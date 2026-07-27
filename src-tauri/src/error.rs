use serde::Serialize;

/// Error every command returns; serializes to `{ code, message }` matching
/// the `IpcError` type in src/lib/ipc.ts.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("TUN mode requires administrator rights")]
    NeedsElevation,
    #[error("sing-box core is not installed")]
    CoreNotInstalled,
    #[error("sing-box failed to start: {0}")]
    CoreStartFailed(String),
    #[error("failed to parse link: {0}")]
    Parse(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("unsupported format: {0}")]
    Unsupported(String),
    /// Panel gates the real server list behind a device id we did not send.
    #[error("this subscription requires a device id (enable \"Send device ID\" in Settings)")]
    HwidRequired,
    /// Panel accepted the device id but the account has no free device slot.
    #[error("subscription device limit reached: unlink a device in the provider's panel")]
    DeviceLimit,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::NeedsElevation => "NEEDS_ELEVATION",
            AppError::CoreNotInstalled => "CORE_NOT_INSTALLED",
            AppError::CoreStartFailed(_) => "CORE_START_FAILED",
            AppError::Parse(_) => "PARSE_ERROR",
            AppError::Network(_) => "NETWORK_ERROR",
            AppError::Unsupported(_) => "UNSUPPORTED_FORMAT",
            AppError::HwidRequired => "HWID_REQUIRED",
            AppError::DeviceLimit => "DEVICE_LIMIT",
            AppError::NotFound(_) => "NOT_FOUND",
            AppError::Io(_) => "IO_ERROR",
            AppError::Internal(_) => "INTERNAL",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Internal(format!("json: {e}"))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Network(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
