use chrono::Utc;
use reqwest::blocking::{Client, Response};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;

const API_ROOT: &str = "https://api.jiahao.chat/api/v1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_GROUP_NAME: &str = "chatgpt";

#[derive(Clone)]
pub struct ApiKeyAssignment {
    pub username: String,
    pub api_key: String,
    pub total_tokens: u64,
    pub(crate) key_id: i64,
    pub(crate) created_date: String,
    pub(crate) panel_token: String,
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    code: i64,
    #[serde(default)]
    message: String,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct LoginData {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct ApiKeyList {
    #[serde(default)]
    items: Vec<ApiKeyRecord>,
}

#[derive(Clone, Debug, Deserialize)]
struct ApiKeyRecord {
    id: i64,
    key: String,
    name: String,
    status: String,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct GroupRecord {
    id: i64,
    name: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct UsageStats {
    #[serde(default)]
    total_tokens: u64,
}

#[derive(Serialize)]
struct LoginRequest<'a> {
    email: &'a str,
    password: &'a str,
}

#[derive(Serialize)]
struct CreateKeyRequest<'a> {
    name: &'a str,
    group_id: i64,
}

pub fn provision(username: &str) -> Result<ApiKeyAssignment, String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("Drive 用户名为空，无法分配 API 密钥".to_owned());
    }

    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("无法初始化 API 密钥服务连接：{error}"))?;
    let panel_token = login(&client)?;
    let mut record = find_key(&client, &panel_token, username)?;
    if record.is_none() {
        let group_id = select_group(&client, &panel_token)?;
        record = Some(create_key(&client, &panel_token, username, group_id)?);
    }
    let record = record.expect("key is assigned above");
    if record.status != "active" {
        return Err(format!(
            "API 密钥“{}”当前状态为 {}，请在 Sub2API 中启用后重试",
            record.name, record.status
        ));
    }

    let created_date = record
        .created_at
        .get(..10)
        .filter(|value| {
            value
                .chars()
                .all(|character| character.is_ascii_digit() || character == '-')
        })
        .unwrap_or("2020-01-01")
        .to_owned();
    let total_tokens = fetch_usage(&client, &panel_token, record.id, &created_date)?;

    Ok(ApiKeyAssignment {
        username: username.to_owned(),
        api_key: record.key,
        total_tokens,
        key_id: record.id,
        created_date,
        panel_token,
    })
}

pub fn refresh_usage(assignment: &ApiKeyAssignment) -> Result<u64, String> {
    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("无法初始化 API 用量连接：{error}"))?;
    match fetch_usage(
        &client,
        &assignment.panel_token,
        assignment.key_id,
        &assignment.created_date,
    ) {
        Ok(total_tokens) => Ok(total_tokens),
        Err(first_error) => {
            let renewed_token = login(&client)?;
            fetch_usage(
                &client,
                &renewed_token,
                assignment.key_id,
                &assignment.created_date,
            )
            .map_err(|retry_error| {
                format!("{retry_error}（重新登录后仍失败；首次错误：{first_error}）")
            })
        }
    }
}

fn login(client: &Client) -> Result<String, String> {
    let response = client
        .post(format!("{API_ROOT}/auth/login"))
        .json(&LoginRequest {
            email: crate::secrets::sub2api_admin_email(),
            password: crate::secrets::sub2api_admin_password(),
        })
        .send()
        .map_err(|error| format!("无法登录 API 密钥服务：{error}"))?;
    let data: LoginData = decode(response, "登录 API 密钥服务")?;
    if data.access_token.trim().is_empty() {
        return Err("API 密钥服务登录成功但未返回访问令牌".to_owned());
    }
    Ok(data.access_token)
}

fn find_key(
    client: &Client,
    panel_token: &str,
    username: &str,
) -> Result<Option<ApiKeyRecord>, String> {
    let response = client
        .get(format!("{API_ROOT}/keys"))
        .bearer_auth(panel_token)
        .query(&[("page", "1"), ("page_size", "1000"), ("search", username)])
        .send()
        .map_err(|error| format!("无法查询 API 密钥：{error}"))?;
    let keys: ApiKeyList = decode(response, "查询 API 密钥")?;
    Ok(find_exact_key(keys.items, username))
}

fn find_exact_key(keys: Vec<ApiKeyRecord>, username: &str) -> Option<ApiKeyRecord> {
    keys.into_iter().find(|key| key.name == username)
}

fn select_group(client: &Client, panel_token: &str) -> Result<i64, String> {
    let response = client
        .get(format!("{API_ROOT}/groups/available"))
        .bearer_auth(panel_token)
        .send()
        .map_err(|error| format!("无法查询 API 分组：{error}"))?;
    let groups: Vec<GroupRecord> = decode(response, "查询 API 分组")?;
    preferred_group_id(groups)
}

fn preferred_group_id(groups: Vec<GroupRecord>) -> Result<i64, String> {
    let mut active = groups
        .into_iter()
        .filter(|group| group.status == "active")
        .collect::<Vec<_>>();
    if let Some(group) = active
        .iter()
        .find(|group| group.name.eq_ignore_ascii_case(DEFAULT_GROUP_NAME))
    {
        return Ok(group.id);
    }
    if active.len() == 1 {
        return Ok(active.remove(0).id);
    }
    Err("未找到唯一可用的 chatgpt API 分组，请先在 Sub2API 中配置".to_owned())
}

fn create_key(
    client: &Client,
    panel_token: &str,
    username: &str,
    group_id: i64,
) -> Result<ApiKeyRecord, String> {
    let response = client
        .post(format!("{API_ROOT}/keys"))
        .bearer_auth(panel_token)
        .header("Idempotency-Key", format!("codex-go-{username}"))
        .json(&CreateKeyRequest {
            name: username,
            group_id,
        })
        .send()
        .map_err(|error| format!("无法为 Drive 用户创建 API 密钥：{error}"))?;
    decode(response, "创建 API 密钥")
}

fn fetch_usage(
    client: &Client,
    panel_token: &str,
    key_id: i64,
    created_date: &str,
) -> Result<u64, String> {
    let end_date = Utc::now().date_naive().format("%Y-%m-%d").to_string();
    let response = client
        .get(format!("{API_ROOT}/usage/stats"))
        .bearer_auth(panel_token)
        .query(&[
            ("api_key_id", key_id.to_string()),
            ("start_date", created_date.to_owned()),
            ("end_date", end_date),
            ("timezone", "Asia/Shanghai".to_owned()),
        ])
        .send()
        .map_err(|error| format!("无法查询 API 密钥 token 用量：{error}"))?;
    let stats: UsageStats = decode(response, "查询 API 密钥 token 用量")?;
    Ok(stats.total_tokens)
}

fn decode<T: DeserializeOwned>(response: Response, action: &str) -> Result<T, String> {
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| format!("{action}时无法读取响应：{error}"))?;
    let envelope = serde_json::from_str::<ApiEnvelope<T>>(&body)
        .map_err(|error| format!("{action}返回了无效响应：{error}"))?;
    if !status.is_success() || envelope.code != 0 {
        let message = if envelope.message.trim().is_empty() {
            status.to_string()
        } else {
            envelope.message
        };
        return Err(format!("{action}失败：{message}"));
    }
    envelope
        .data
        .ok_or_else(|| format!("{action}成功但响应中没有数据"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_root_uses_the_configured_https_service() {
        assert_eq!(API_ROOT, "https://api.jiahao.chat/api/v1");
    }

    #[test]
    fn assignment_clone_keeps_public_usage_fields() {
        let assignment = ApiKeyAssignment {
            username: "jiahao".to_owned(),
            api_key: "sk-test".to_owned(),
            total_tokens: 1_250_000,
            key_id: 2,
            created_date: "2026-01-01".to_owned(),
            panel_token: "panel-test".to_owned(),
        };
        let cloned = assignment.clone();
        assert_eq!(cloned.username, "jiahao");
        assert_eq!(cloned.total_tokens, 1_250_000);
    }

    #[test]
    fn key_matching_is_exact_and_case_sensitive() {
        let keys = vec![
            ApiKeyRecord {
                id: 1,
                key: "sk-kitty".to_owned(),
                name: "kitty".to_owned(),
                status: "active".to_owned(),
                created_at: "2026-01-01T00:00:00+08:00".to_owned(),
            },
            ApiKeyRecord {
                id: 2,
                key: "sk-jiahao".to_owned(),
                name: "jiahao".to_owned(),
                status: "active".to_owned(),
                created_at: "2026-01-01T00:00:00+08:00".to_owned(),
            },
        ];

        assert_eq!(find_exact_key(keys.clone(), "jiahao").unwrap().id, 2);
        assert!(find_exact_key(keys, "Jiahao").is_none());
    }

    #[test]
    fn chatgpt_group_wins_and_single_active_group_is_the_fallback() {
        let groups = vec![
            GroupRecord {
                id: 1,
                name: "other".to_owned(),
                status: "active".to_owned(),
            },
            GroupRecord {
                id: 2,
                name: "ChatGPT".to_owned(),
                status: "active".to_owned(),
            },
        ];
        assert_eq!(preferred_group_id(groups).unwrap(), 2);

        let single = vec![GroupRecord {
            id: 3,
            name: "future-default".to_owned(),
            status: "active".to_owned(),
        }];
        assert_eq!(preferred_group_id(single).unwrap(), 3);
    }
}
