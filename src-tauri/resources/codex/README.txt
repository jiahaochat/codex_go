install.ps1 is not committed to this repository.
Run scripts/prepare-windows-assets.ps1 before a Windows package build. The
script downloads the installer from a pinned OpenAI Codex commit and verifies
its SHA-256 digest before packaging it.
