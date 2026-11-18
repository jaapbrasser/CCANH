//! Opens upstream sources for the stream manager through the camera's provider.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

use super::manager::{SourceOpener, StreamKey};
use super::media::MediaSource;
use crate::cameras;
use crate::config::Mode;
use crate::credentials::CredentialStore;
use crate::providers::{ProviderError, ProviderFactory};
use crate::settings::SettingsHandle;

pub struct ProviderOpener {
    pub db: SqlitePool,
    pub store: Arc<dyn CredentialStore>,
    pub settings: SettingsHandle,
    pub mode: Mode,
}

#[async_trait]
impl SourceOpener for ProviderOpener {
    async fn open(
        &self,
        key: &StreamKey,
        cancel: CancellationToken,
    ) -> Result<Box<dyn MediaSource>, ProviderError> {
        let target = cameras::target(&self.db, key.camera_id)
            .await
            .map_err(|e| ProviderError::Internal(format!("database: {e}")))?
            .ok_or_else(|| ProviderError::unsupported("this camera no longer exists"))?
            .provider_target(&*self.store)
            .map_err(|e| ProviderError::Internal(format!("credential store: {e}")))?;
        let timeout = Duration::from_secs(self.settings.current().monitoring.timeout_secs);
        let provider = ProviderFactory::new(self.mode, timeout).create(&target);
        provider.open_stream(&key.profile, cancel).await
    }
}
