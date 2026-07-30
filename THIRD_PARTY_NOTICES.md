# Third-party notices

## Xray-core

`codex_go` packages the unmodified `xray.exe` from XTLS/Xray-core as a separate
process for the scoped download proxy.

- Project: https://github.com/XTLS/Xray-core
- Version: `v26.3.27`
- Windows x64 archive SHA-256: `d004c39288ce9ada487c6f398c7c545f7d749e44bdfdd59dbc9f865afba4e1ad`
- Packaged `xray.exe` SHA-256: `15c2d007954ac53ba69b80ec91242786b3c0b71d52649165b4ca1d5cc96ef8f1`
- Source: https://github.com/XTLS/Xray-core/tree/v26.3.27
- License: Mozilla Public License 2.0

The upstream `LICENSE` file is copied beside `xray.exe` during packaging as
`LICENSE-XRAY.txt`.

## OpenAI Codex installer

The Windows package contains an unmodified copy of OpenAI Codex's official
standalone installer script. It downloads Codex from OpenAI on the user's
device; `codex_go` does not redistribute the Codex binary.

- Project: https://github.com/openai/codex
- Commit: `6219b7c40fc9c702c0aef9964e72b492558f60e4`
- Script SHA-256: `391f247de2c70c7e99041979ec02dae7e76be27ac9cfc1dfe7c1eb21d48d8b97`
- License: Apache License 2.0

The upstream Apache 2.0 `LICENSE` and `NOTICE` files are packaged beside the
installer script as `LICENSE-APACHE-2.0.txt` and `NOTICE-CODEX.txt`.
