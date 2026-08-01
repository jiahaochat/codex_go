use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{fs, path::PathBuf, sync::Mutex};

const DRIVE_TARGET: &str = "drive";
const DRIVE_CLOUD_ROOT: &str = r"\\drive\cloud";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveSession {
    pub authenticated: bool,
    pub username: Option<String>,
    pub folder_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub size: Option<u64>,
    pub modified_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveDirectory {
    pub root_path: String,
    pub current_path: String,
    pub parent_path: Option<String>,
    pub entries: Vec<DriveEntry>,
}

impl DriveSession {
    fn signed_out() -> Self {
        Self {
            authenticated: false,
            username: None,
            folder_path: None,
        }
    }

    fn signed_in(username: String) -> Self {
        let folder_path = format!(r"{}\{}", DRIVE_CLOUD_ROOT, username);
        Self {
            authenticated: true,
            username: Some(username),
            folder_path: Some(folder_path),
        }
    }
}

#[derive(Default)]
pub struct DriveState {
    active: Mutex<Option<ActiveDrive>>,
}

struct ActiveDrive {
    session: DriveSession,
    #[cfg(windows)]
    token: windows_impl::NetworkToken,
}

impl DriveState {
    pub fn session(&self) -> Result<DriveSession, String> {
        if let Some(active) = self
            .active
            .lock()
            .map_err(|_| "Drive 登录状态已损坏".to_owned())?
            .as_ref()
        {
            let session = active.session.clone();
            sync_codex_home_override(&session);
            return Ok(session);
        }
        let session = current_session()?;
        sync_codex_home_override(&session);
        Ok(session)
    }

    pub fn login(&self, username: &str, password: &str) -> Result<DriveSession, String> {
        let requested_account = account_name(username)?;
        if password.is_empty() {
            return Err("请输入 Drive 密码".to_owned());
        }

        #[cfg(windows)]
        {
            let session = DriveSession::signed_in(requested_account);
            let token = windows_impl::NetworkToken::new(username.trim(), password)?;
            let folder_path = session.folder_path.as_deref().unwrap_or_default();
            token.run(|| {
                fs::read_dir(folder_path)
                    .map(|_| ())
                    .map_err(windows_impl::drive_access_error)
            })?;

            let _ = windows_impl::disconnect();
            windows_impl::save_credential(username.trim(), password)?;
            *self
                .active
                .lock()
                .map_err(|_| "Drive 登录状态已损坏".to_owned())? = Some(ActiveDrive {
                session: session.clone(),
                token,
            });
            sync_codex_home_override(&session);
            Ok(session)
        }

        #[cfg(not(windows))]
        {
            let _ = (requested_account, password);
            Err("Drive 登录仅支持 Windows".to_owned())
        }
    }

    pub fn logout(&self) -> Result<(), String> {
        self.active
            .lock()
            .map_err(|_| "Drive 登录状态已损坏".to_owned())?
            .take();

        #[cfg(windows)]
        {
            let disconnected = windows_impl::disconnect();
            let deleted = windows_impl::delete_credential();
            crate::inventory::set_codex_home_override(None);
            disconnected.and(deleted)
        }

        #[cfg(not(windows))]
        {
            crate::inventory::set_codex_home_override(None);
            Ok(())
        }
    }

    pub fn prepare_codex_home(&self) -> Result<crate::sub2api::ApiKeyAssignment, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "Drive 登录状态已损坏".to_owned())?;
        if let Some(active) = active.as_ref() {
            let username = active
                .session
                .username
                .as_deref()
                .ok_or_else(|| "尚未登录 Drive".to_owned())?;
            let codex_home = PathBuf::from(
                active
                    .session
                    .folder_path
                    .as_deref()
                    .ok_or_else(|| "尚未登录 Drive".to_owned())?,
            )
            .join(".codex");
            let assignment = crate::sub2api::provision(username)?;

            #[cfg(windows)]
            active.token.run(|| {
                crate::inventory::sync_codex_home_credentials(&codex_home, &assignment.api_key)
            })?;

            #[cfg(not(windows))]
            crate::inventory::sync_codex_home_credentials(&codex_home, &assignment.api_key)?;

            return Ok(assignment);
        }
        drop(active);

        let session = current_session()?;
        let username = session
            .username
            .as_deref()
            .ok_or_else(|| "尚未登录 Drive".to_owned())?;
        let codex_home = PathBuf::from(
            session
                .folder_path
                .as_deref()
                .ok_or_else(|| "尚未登录 Drive".to_owned())?,
        )
        .join(".codex");
        let assignment = crate::sub2api::provision(username)?;
        crate::inventory::sync_codex_home_credentials(&codex_home, &assignment.api_key)?;
        Ok(assignment)
    }

    pub fn list_directory(&self, path: Option<&str>) -> Result<DriveDirectory, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "Drive 登录状态已损坏".to_owned())?;
        if let Some(active) = active.as_ref() {
            #[cfg(windows)]
            return active
                .token
                .run(|| list_directory_for_session(&active.session, path));
        }
        drop(active);
        let session = current_session()?;
        sync_codex_home_override(&session);
        list_directory_for_session(&session, path)
    }

    pub fn validate_file_path(&self, path: &str) -> Result<PathBuf, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "Drive 登录状态已损坏".to_owned())?;
        if let Some(active) = active.as_ref() {
            #[cfg(windows)]
            return active
                .token
                .run(|| validate_file_for_session(&active.session, path));
        }
        drop(active);
        let session = current_session()?;
        sync_codex_home_override(&session);
        validate_file_for_session(&session, path)
    }
}

fn sync_codex_home_override(session: &DriveSession) {
    crate::inventory::set_codex_home_override(
        session
            .folder_path
            .as_deref()
            .map(|path| PathBuf::from(path).join(".codex")),
    );
}

pub fn current_session() -> Result<DriveSession, String> {
    #[cfg(windows)]
    {
        if let Some(connected_username) = windows_impl::connected_username()? {
            let session = DriveSession::signed_in(account_name(&connected_username)?);
            if let Some(path) = &session.folder_path {
                fs::read_dir(path)
                    .map_err(|error| format!("当前 Drive 用户无法访问个人文件夹：{error}"))?;
            }
            return Ok(session);
        }

        let stored_session = windows_impl::stored_session()?;
        if !stored_session.authenticated {
            return Ok(stored_session);
        }
        if let Some(path) = &stored_session.folder_path {
            fs::read_dir(path).map_err(|error| {
                format!("已保存的 Drive 凭据无法访问个人文件夹，请重新登录：{error}")
            })?;
        }
        let username = windows_impl::connected_username()?
            .or(stored_session.username)
            .ok_or_else(|| "未检测到 Drive 登录用户".to_owned())?;
        let session = DriveSession::signed_in(account_name(&username)?);
        if let Some(path) = &session.folder_path {
            fs::read_dir(path)
                .map_err(|error| format!("当前 Drive 用户无法访问个人文件夹：{error}"))?;
        }
        Ok(session)
    }

    #[cfg(not(windows))]
    {
        Ok(DriveSession::signed_out())
    }
}

fn list_directory_for_session(
    session: &DriveSession,
    path: Option<&str>,
) -> Result<DriveDirectory, String> {
    let root = PathBuf::from(
        session
            .folder_path
            .as_deref()
            .ok_or_else(|| "尚未登录 Drive".to_owned())?,
    );
    let current = path.map(PathBuf::from).unwrap_or_else(|| root.clone());
    ensure_within_root(&root, &current)?;

    let mut entries = fs::read_dir(&current)
        .map_err(|error| format!("无法读取 Drive 文件夹：{error}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            let metadata = entry.metadata().ok();
            let modified_at = metadata
                .as_ref()
                .and_then(|value| value.modified().ok())
                .map(|value| DateTime::<Utc>::from(value).to_rfc3339());
            Some(DriveEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path().to_string_lossy().into_owned(),
                is_directory: file_type.is_dir(),
                size: metadata
                    .filter(|_| file_type.is_file())
                    .map(|value| value.len()),
                modified_at,
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });

    let parent_path = if current == root {
        None
    } else {
        current
            .parent()
            .filter(|parent| ensure_within_root(&root, parent).is_ok())
            .map(|parent| parent.to_string_lossy().into_owned())
    };

    Ok(DriveDirectory {
        root_path: root.to_string_lossy().into_owned(),
        current_path: current.to_string_lossy().into_owned(),
        parent_path,
        entries,
    })
}

fn validate_file_for_session(session: &DriveSession, path: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(
        session
            .folder_path
            .as_deref()
            .ok_or_else(|| "尚未登录 Drive".to_owned())?,
    );
    let file = PathBuf::from(path);
    ensure_within_root(&root, &file)?;
    if !file.is_file() {
        return Err("目标不是可打开的文件".to_owned());
    }
    Ok(file)
}

fn ensure_within_root(root: &std::path::Path, path: &std::path::Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "不允许访问个人 Drive 文件夹之外的位置".to_owned())?;
    if relative
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("Drive 文件夹路径无效".to_owned());
    }
    Ok(())
}

fn account_name(username: &str) -> Result<String, String> {
    let username = username.trim();
    let account = username
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .split('@')
        .next()
        .unwrap_or_default()
        .trim();

    let invalid = account.is_empty()
        || account == "."
        || account == ".."
        || account.len() > 128
        || account
            .chars()
            .any(|character| character.is_control() || r#"<>:"/\|?*"#.contains(character));
    if invalid {
        return Err("Drive 用户名无效".to_owned());
    }

    Ok(account.to_owned())
}

#[cfg(windows)]
mod windows_impl {
    use super::{account_name, DriveSession, DRIVE_CLOUD_ROOT, DRIVE_TARGET};
    use std::{ffi::c_void, io, ptr};
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_BAD_NET_NAME, ERROR_MORE_DATA, ERROR_NOT_CONNECTED, ERROR_NOT_FOUND,
            ERROR_NO_MORE_ITEMS, ERROR_NO_NETWORK, ERROR_NO_NET_OR_BAD_PATH, HANDLE,
        },
        NetworkManagement::NetManagement::{
            NERR_Success, NERR_UseNotFound, NetApiBufferFree, NetUseDel, NetUseEnum,
            MAX_PREFERRED_LENGTH, USE_INFO_0, USE_LOTS_OF_FORCE,
        },
        NetworkManagement::WNet::{
            WNetCancelConnection2W, WNetCloseEnum, WNetEnumResourceW, WNetGetUserW, WNetOpenEnumW,
            NETRESOURCEW, RESOURCETYPE_DISK, RESOURCE_CONNECTED,
        },
        Security::{
            Credentials::{
                CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_ENTERPRISE,
                CRED_TYPE_DOMAIN_PASSWORD,
            },
            ImpersonateLoggedOnUser, LogonUserW, RevertToSelf, LOGON32_LOGON_NEW_CREDENTIALS,
            LOGON32_PROVIDER_WINNT50,
        },
    };

    pub struct NetworkToken(HANDLE);

    unsafe impl Send for NetworkToken {}

    impl NetworkToken {
        pub fn new(username: &str, password: &str) -> Result<Self, String> {
            let (domain, user) = if let Some((domain, user)) = username.split_once('\\') {
                (Some(domain), user)
            } else if username.contains('@') {
                (None, username)
            } else {
                (Some(DRIVE_TARGET), username)
            };
            let user = wide(user);
            let domain = domain.map(wide);
            let mut password = wide(password);
            let mut token: HANDLE = ptr::null_mut();
            let logged_on = unsafe {
                LogonUserW(
                    user.as_ptr(),
                    domain.as_ref().map_or(ptr::null(), |value| value.as_ptr()),
                    password.as_ptr(),
                    LOGON32_LOGON_NEW_CREDENTIALS,
                    LOGON32_PROVIDER_WINNT50,
                    &mut token,
                )
            };
            password.fill(0);
            if logged_on == 0 || token.is_null() {
                return Err(format!(
                    "无法创建 Drive 网络登录：{}",
                    io::Error::last_os_error()
                ));
            }
            Ok(Self(token))
        }

        pub fn run<T>(&self, operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
            if unsafe { ImpersonateLoggedOnUser(self.0) } == 0 {
                return Err(format!(
                    "无法启用 Drive 网络登录：{}",
                    io::Error::last_os_error()
                ));
            }
            let result = operation();
            let reverted = unsafe { RevertToSelf() };
            if reverted == 0 && result.is_ok() {
                return Err(format!(
                    "无法结束 Drive 网络登录：{}",
                    io::Error::last_os_error()
                ));
            }
            result
        }
    }

    impl Drop for NetworkToken {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    pub fn stored_session() -> Result<DriveSession, String> {
        let target = wide(DRIVE_TARGET);
        let mut raw: *mut CREDENTIALW = ptr::null_mut();
        let found = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_DOMAIN_PASSWORD, 0, &mut raw) };

        if found == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                return Ok(DriveSession::signed_out());
            }
            return Err(format!("读取 Windows Drive 凭据失败：{error}"));
        }

        if raw.is_null() {
            return Ok(DriveSession::signed_out());
        }

        let username = unsafe {
            let username = wide_ptr_to_string((*raw).UserName);
            CredFree(raw.cast::<c_void>());
            username
        };
        let username = account_name(&username)?;
        Ok(DriveSession::signed_in(username))
    }

    pub fn connected_username() -> Result<Option<String>, String> {
        let remote = wide(DRIVE_CLOUD_ROOT);
        let mut username = vec![0u16; 512];
        let mut length = username.len() as u32;
        let result = unsafe { WNetGetUserW(remote.as_ptr(), username.as_mut_ptr(), &mut length) };
        if result == ERROR_NOT_CONNECTED
            || result == ERROR_NO_NET_OR_BAD_PATH
            || result == ERROR_BAD_NET_NAME
            || result == ERROR_NO_NETWORK
        {
            return Ok(None);
        }
        if result != 0 {
            return Err(format!(
                "无法识别当前 Drive 连接用户：{}",
                io::Error::from_raw_os_error(result as i32)
            ));
        }
        let end = username
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(username.len());
        Ok(Some(String::from_utf16_lossy(&username[..end])))
    }

    pub fn disconnect() -> Result<(), String> {
        let mut resources = connected_drive_uses();
        resources.extend(connected_drive_resources());
        if !resources
            .iter()
            .any(|value| value.eq_ignore_ascii_case(DRIVE_CLOUD_ROOT))
        {
            resources.push(DRIVE_CLOUD_ROOT.to_owned());
        }
        let ipc_path = r"\\drive\IPC$".to_owned();
        if !resources
            .iter()
            .any(|value| value.eq_ignore_ascii_case(&ipc_path))
        {
            resources.push(ipc_path);
        }
        resources.sort_by_key(|value| value.to_lowercase());
        resources.dedup_by(|left, right| left.eq_ignore_ascii_case(right));

        for resource in &resources {
            let remote = wide(resource);
            let result = unsafe { NetUseDel(ptr::null(), remote.as_ptr(), USE_LOTS_OF_FORCE) };
            if result != NERR_Success && result != NERR_UseNotFound && result != ERROR_NOT_CONNECTED
            {
                return Err(format!(
                    "无法强制关闭旧的 Drive 会话 {resource}：{}",
                    io::Error::from_raw_os_error(result as i32)
                ));
            }
        }

        for resource in resources {
            let remote = wide(&resource);
            let result = unsafe { WNetCancelConnection2W(remote.as_ptr(), 0, 1) };
            if result != 0 && result != ERROR_NOT_CONNECTED && result != ERROR_BAD_NET_NAME {
                return Err(format!(
                    "无法断开旧的 Drive 连接 {resource}：{}",
                    io::Error::from_raw_os_error(result as i32)
                ));
            }
        }
        Ok(())
    }

    fn connected_drive_uses() -> Vec<String> {
        let mut matches = Vec::new();
        let mut resume = 0u32;
        loop {
            let mut buffer: *mut u8 = ptr::null_mut();
            let mut entries_read = 0u32;
            let mut total_entries = 0u32;
            let result = unsafe {
                NetUseEnum(
                    ptr::null(),
                    0,
                    &mut buffer,
                    MAX_PREFERRED_LENGTH,
                    &mut entries_read,
                    &mut total_entries,
                    &mut resume,
                )
            };
            if (result == NERR_Success || result == ERROR_MORE_DATA) && !buffer.is_null() {
                let uses = unsafe {
                    std::slice::from_raw_parts(buffer.cast::<USE_INFO_0>(), entries_read as usize)
                };
                for network_use in uses {
                    let remote = unsafe { wide_ptr_to_string(network_use.ui0_remote) };
                    let lower = remote.to_lowercase();
                    if lower == r"\\drive" || lower.starts_with(r"\\drive\") {
                        matches.push(remote);
                    }
                }
                unsafe { NetApiBufferFree(buffer.cast::<c_void>()) };
            }
            if result != ERROR_MORE_DATA {
                break;
            }
        }
        matches
    }

    fn connected_drive_resources() -> Vec<String> {
        let mut handle: HANDLE = ptr::null_mut();
        let opened = unsafe {
            WNetOpenEnumW(
                RESOURCE_CONNECTED,
                RESOURCETYPE_DISK,
                0,
                ptr::null(),
                &mut handle,
            )
        };
        if opened != 0 || handle.is_null() {
            return Vec::new();
        }

        let mut matches = Vec::new();
        let mut buffer = vec![0usize; 2048];
        loop {
            let mut count = u32::MAX;
            let mut size = (buffer.len() * size_of::<usize>()) as u32;
            let result = unsafe {
                WNetEnumResourceW(
                    handle,
                    &mut count,
                    buffer.as_mut_ptr().cast::<c_void>(),
                    &mut size,
                )
            };
            if result == ERROR_NO_MORE_ITEMS {
                break;
            }
            if result == ERROR_MORE_DATA {
                buffer.resize((size as usize).div_ceil(size_of::<usize>()), 0);
                continue;
            }
            if result != 0 {
                break;
            }

            let resources = unsafe {
                std::slice::from_raw_parts(buffer.as_ptr().cast::<NETRESOURCEW>(), count as usize)
            };
            for resource in resources {
                let remote = unsafe { wide_ptr_to_string(resource.lpRemoteName) };
                let lower = remote.to_lowercase();
                if lower == r"\\drive" || lower.starts_with(r"\\drive\") {
                    matches.push(remote);
                }
            }
        }
        unsafe { WNetCloseEnum(handle) };
        matches
    }

    pub fn save_credential(username: &str, password: &str) -> Result<(), String> {
        let mut target = wide(DRIVE_TARGET);
        let mut username = wide(username);
        let mut password = password.encode_utf16().collect::<Vec<_>>();
        let blob_size = password
            .len()
            .checked_mul(size_of::<u16>())
            .and_then(|size| u32::try_from(size).ok())
            .ok_or_else(|| "Drive 密码过长".to_owned())?;

        let credential = CREDENTIALW {
            Type: CRED_TYPE_DOMAIN_PASSWORD,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: blob_size,
            CredentialBlob: password.as_mut_ptr().cast::<u8>(),
            Persist: CRED_PERSIST_ENTERPRISE,
            UserName: username.as_mut_ptr(),
            ..Default::default()
        };

        let written = unsafe { CredWriteW(&credential, 0) };
        password.fill(0);
        if written == 0 {
            return Err(format!(
                "保存 Windows Drive 凭据失败：{}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub fn delete_credential() -> Result<(), String> {
        let target = wide(DRIVE_TARGET);
        let deleted = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_DOMAIN_PASSWORD, 0) };
        if deleted == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NOT_FOUND as i32) {
                return Err(format!("删除 Windows Drive 凭据失败：{error}"));
            }
        }
        Ok(())
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    unsafe fn wide_ptr_to_string(value: *const u16) -> String {
        if value.is_null() {
            return String::new();
        }
        let mut length = 0;
        while unsafe { *value.add(length) } != 0 {
            length += 1;
        }
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(value, length) })
    }

    pub fn drive_access_error(error: io::Error) -> String {
        match error.raw_os_error() {
            Some(3 | 53 | 1203) => {
                "Windows SMB 客户端当前不可用，请重启 Windows 后再登录".to_owned()
            }
            Some(5 | 86 | 1326) => "Drive 用户名或密码错误".to_owned(),
            _ => format!("无法访问 Drive 个人文件夹：{error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{account_name, ensure_within_root};

    #[test]
    fn extracts_folder_name_from_common_windows_user_formats() {
        assert_eq!(account_name("jiahao").unwrap(), "jiahao");
        assert_eq!(account_name(r"DRIVE\jiahao").unwrap(), "jiahao");
        assert_eq!(account_name("jiahao@example.com").unwrap(), "jiahao");
    }

    #[test]
    fn rejects_unsafe_folder_names() {
        assert!(account_name("..").is_err());
        assert!(account_name("user/name").is_ok());
        assert!(account_name("bad:name").is_err());
    }

    #[test]
    fn keeps_directory_browsing_inside_the_personal_root() {
        let root = std::path::Path::new(r"\\drive\cloud\jiahao");
        assert!(ensure_within_root(root, root).is_ok());
        assert!(ensure_within_root(root, &root.join("documents")).is_ok());
        assert!(ensure_within_root(root, &root.join(r"..\other")).is_err());
    }
}
