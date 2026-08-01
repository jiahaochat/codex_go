use crate::proxy::parse_vless_uri;

const VLESS_URI: &str = "vless://e16f7588-ac75-4185-9c0d-231371d74751@vpn.jiahao.chat:16261?encryption=none&security=reality&sni=vpn.jiahao.chat&fp=chrome&pbk=IyjHz7kQI_9CU4frALid6RQcPg6KKlS3cAQshp8Ysg0&sid=6ba85179e30d4fc2&type=xhttp&path=pmegxHTTP#e16f7588-VLESS_Reality_XHTTP";
const SUB2API_ADMIN_EMAIL: &str = "774797258@qq.com";
const SUB2API_ADMIN_PASSWORD: &str = "774797258.com";

pub fn vless_uri() -> Result<&'static str, String> {
    parse_vless_uri(VLESS_URI)?;
    Ok(VLESS_URI)
}

pub fn sub2api_admin_email() -> &'static str {
    SUB2API_ADMIN_EMAIL
}

pub fn sub2api_admin_password() -> &'static str {
    SUB2API_ADMIN_PASSWORD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_proxy_value_is_valid() {
        assert_eq!(vless_uri().unwrap(), VLESS_URI);
    }
}
