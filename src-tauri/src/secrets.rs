use serde::Serialize;

use crate::proxy::parse_vless_uri;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxySource {
    Build,
    Environment,
    None,
}

pub struct ResolvedSecret {
    pub uri: String,
    pub source: ProxySource,
}

pub fn resolve_vless_uri() -> Result<Option<ResolvedSecret>, String> {
    if let Ok(value) = std::env::var("CODEX_GO_VLESS_URI") {
        if let Some(uri) = normalize_vless_uri(&value)? {
            return Ok(Some(ResolvedSecret {
                uri,
                source: ProxySource::Environment,
            }));
        }
    }

    if let Some(value) = option_env!("CODEX_GO_DEFAULT_VLESS_URI") {
        if let Some(uri) = normalize_vless_uri(value)? {
            return Ok(Some(ResolvedSecret {
                uri,
                source: ProxySource::Build,
            }));
        }
    }

    Ok(None)
}

fn normalize_vless_uri(value: &str) -> Result<Option<String>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    parse_vless_uri(value)?;
    Ok(Some(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_source_serializes_for_frontend() {
        assert_eq!(
            serde_json::to_string(&ProxySource::Build).unwrap(),
            "\"build\""
        );
    }

    #[test]
    fn proxy_values_are_trimmed_before_storage_or_use() {
        let uri = "vless://11111111-2222-4333-8444-555555555555@example.com:443?encryption=none&security=reality&sni=cdn.example.com&fp=chrome&pbk=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&sid=0123456789abcdef&type=xhttp&path=test";
        assert_eq!(
            normalize_vless_uri(&format!(" \r\n{uri}\t"))
                .unwrap()
                .as_deref(),
            Some(uri)
        );
        assert_eq!(normalize_vless_uri(" \r\n ").unwrap(), None);
    }

    #[test]
    fn embedded_proxy_value_is_valid_when_configured() {
        if let Some(uri) = option_env!("CODEX_GO_DEFAULT_VLESS_URI") {
            assert!(normalize_vless_uri(uri).is_ok());
        }
    }
}
