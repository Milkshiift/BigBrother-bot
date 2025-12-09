use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProcessorError {
	#[error("Recoverable error: {0}")]
	Recoverable(anyhow::Error),
	#[error("Fatal error: {0}")]
	Fatal(anyhow::Error),
}

impl From<anyhow::Error> for ProcessorError {
	fn from(err: anyhow::Error) -> Self {
		// Serialization Errors
		// If we can't serialize the data to JSON, retrying won't fix it.
		// This is a logic bug or data corruption.
		if err.is::<sonic_rs::Error>() {
			return Self::Fatal(err);
		}

		// I/O Errors
		if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
			match io_err.kind() {
				std::io::ErrorKind::PermissionDenied |
				std::io::ErrorKind::WriteZero | // Usually indicates disk full
				std::io::ErrorKind::NotFound => return Self::Fatal(err),
				_ => {
					if let Some(code) = io_err.raw_os_error()
                // 28: ENOSPC (No space left on device)
                // 30: EROFS (Read-only file system)
                // 5:  EIO (Input/output error - hardware failure)
                && code == 28 | 30 | 5
					{
						return Self::Fatal(err);
					}
				}
			}
		}

		// HTTP Errors
		if let Some(http_err) = err.downcast_ref::<twilight_http::Error>()
			&& let twilight_http::error::ErrorType::Response { status, .. } = http_err.kind()
		{
			match status.get() {
				// 401: Unauthorized (Token invalid)
				// 403: Forbidden (Missing permissions/Intents)
				// 405: Method Not Allowed (API usage error)
				401 | 403 | 405 => return Self::Fatal(err),
				_ => {}
			}
		}

		// Consider everything else transient
		Self::Recoverable(err)
	}
}
