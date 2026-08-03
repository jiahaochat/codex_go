use mysql::{prelude::Queryable, Opts, Pool};

const ACCOUNT_COLUMNS: &[&str] = &["username", "user_name", "account", "name", "email"];

pub fn user_avatar(username: &str) -> Result<Option<String>, String> {
    let options = Opts::from_url(crate::secrets::profile_database_url())
        .map_err(|error| format!("头像数据库配置无效：{error}"))?;
    let pool = Pool::new(options).map_err(|error| format!("无法连接头像数据库：{error}"))?;
    let mut connection = pool
        .get_conn()
        .map_err(|error| format!("无法连接头像数据库：{error}"))?;
    let columns = connection
        .exec_map::<String, _, _, _, _>(
            "SELECT COLUMN_NAME FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'user'",
            (),
            |field| field,
        )
        .map_err(|error| format!("无法读取头像表结构：{error}"))?;
    if !columns.iter().any(|column| column == "wx_avatar") {
        return Ok(None);
    }
    let Some(account_column) = ACCOUNT_COLUMNS
        .iter()
        .find(|candidate| columns.iter().any(|column| column == **candidate))
    else {
        return Ok(None);
    };
    let query = format!("SELECT `wx_avatar` FROM `user` WHERE `{account_column}` = ? LIMIT 1");
    let avatar: Option<String> = connection
        .exec_first(query, (username,))
        .map_err(|error| format!("无法读取用户头像：{error}"))?;
    Ok(avatar.filter(|value| !value.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_column_candidates_are_fixed_identifiers() {
        assert!(ACCOUNT_COLUMNS.contains(&"username"));
        assert!(ACCOUNT_COLUMNS.iter().all(|column| column
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '_')));
    }
}
