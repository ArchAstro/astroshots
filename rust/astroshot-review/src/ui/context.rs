//! Port of `packages/astroshot-review/src/ui/context.tsx`.
//!
//! # React context -> explicit handle
//!
//! `ServicesContext` / `useServices()` become [`AppServices`], built once in
//! `cli` and passed by reference to every screen (`&AppServices`). It is
//! cheap to clone (everything inside is an `Arc` or a small value), so the
//! app state struct can own one and hand `&services` to children.
//!
//! How the later ports consume it (checked against `app.tsx`, `stream.tsx`,
//! `detail.tsx`, `settings.tsx`):
//!
//! - `app.tsx` destructures `{ store, layer, capabilities }`: the app state
//!   struct owns the `AppServices`, a [`hooks::StoreStateHook`],
//!   [`hooks::TerminalSizeHook`] and [`hooks::ClockHook`], all built from it.
//! - `stream.tsx` / `detail.tsx` read `capabilities` and `ffmpeg` straight off
//!   `&AppServices`, and each `<Picture>` becomes a keyed
//!   [`picture::PictureState`] built with `PictureState::new(&services, wake)`.
//!   The parent keeps one per visible shot (a `HashMap` keyed like the React
//!   `key`) and drops the ones that scroll away; dropping unregisters the
//!   image from the layer.
//! - `settings.tsx` reads `capabilities`, `ffmpeg`, `roots_source`, `version`.
//!
//! `use_services` is kept for the "context missing" error path when a screen
//! holds an `Option<&AppServices>`.
//!
//! [`hooks::StoreStateHook`]: super::hooks::StoreStateHook
//! [`hooks::TerminalSizeHook`]: super::hooks::TerminalSizeHook
//! [`hooks::ClockHook`]: super::hooks::ClockHook
//! [`picture::PictureState`]: super::picture::PictureState

use std::sync::Arc;

use crate::data::store::ReviewStore;
use crate::images::service::ImageService;
use crate::terminal::image_layer::ImageLayer;
use crate::terminal::probe::TerminalCapabilities;
use crate::video::ffmpeg::FfmpegInfo;

/// `rootsSource: "app" | "cli" | "cwd"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootsSource {
    App,
    Cli,
    Cwd,
}

impl RootsSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RootsSource::App => "app",
            RootsSource::Cli => "cli",
            RootsSource::Cwd => "cwd",
        }
    }
}

#[derive(Clone)]
pub struct AppServices {
    pub store: Arc<ReviewStore>,
    pub layer: ImageLayer,
    pub service: Arc<dyn ImageService>,
    pub capabilities: TerminalCapabilities,
    pub ffmpeg: FfmpegInfo,
    pub roots_source: RootsSource,
    pub version: String,
    /// The command that starts the tray, shown in hints and the settings
    /// header (`astroshot review` standalone; a host passes its own).
    pub command: String,
}

/// `useServices()`: `services` is the context value, `None` when no provider wrapped the tree.
pub fn use_services(services: Option<&AppServices>) -> anyhow::Result<&AppServices> {
    services.ok_or_else(|| anyhow::anyhow!("ServicesContext is missing"))
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::images::png::ImageSize;
    use crate::images::scale::{Rect, ScaledFormat};
    use crate::images::service::{PrepareFuture, PreparedImage};
    use crate::terminal::image_layer::ImageLayerOptions;
    use crate::terminal::probe::{CellSource, GraphicsProtocol};
    use std::sync::Mutex;

    pub type Request = (String, ImageSize, ScaledFormat);

    /// Resolves with a solid red top half and blue bottom half at the requested size.
    pub struct FakeService {
        pub fail: bool,
        pub requests: Mutex<Vec<Request>>,
    }

    impl FakeService {
        pub fn new(fail: bool) -> Arc<Self> {
            Arc::new(Self {
                fail,
                requests: Mutex::new(Vec::new()),
            })
        }
    }

    impl ImageService for FakeService {
        fn prepare(
            &self,
            file_path: String,
            target: ImageSize,
            format: ScaledFormat,
            _crop: Option<Rect>,
        ) -> PrepareFuture {
            self.requests
                .lock()
                .unwrap()
                .push((file_path.clone(), target, format));
            let result = if self.fail {
                Err(anyhow::anyhow!("decode failed"))
            } else {
                let mut data = Vec::new();
                for y in 0..target.height {
                    for _ in 0..target.width {
                        if y < target.height / 2 {
                            data.extend_from_slice(&[255, 0, 0]);
                        } else {
                            data.extend_from_slice(&[0, 0, 255]);
                        }
                    }
                }
                Ok(Arc::new(PreparedImage {
                    key: file_path.clone(),
                    path: file_path,
                    width: target.width,
                    height: target.height,
                    source_width: target.width,
                    source_height: target.height,
                    format,
                    data,
                    is_original: false,
                    mtime_ms: 0.0,
                    size: 0,
                }))
            };
            Box::pin(async move { result })
        }
    }

    pub fn capabilities(graphics: GraphicsProtocol) -> TerminalCapabilities {
        TerminalCapabilities {
            graphics,
            file_medium: false,
            cell_width: 10,
            cell_height: 20,
            cell_source: CellSource::Env,
            reason: None,
            inside_tmux: false,
            inside_ssh: false,
            inside_mosh: false,
            inside_herdr: false,
            intercepted: None,
        }
    }

    /// Must run inside a tokio runtime (the layer captures the current one).
    pub fn services(graphics: GraphicsProtocol, service: Arc<FakeService>) -> AppServices {
        let caps = capabilities(graphics);
        let layer = ImageLayer::new(ImageLayerOptions::new(
            caps.clone(),
            service.clone(),
            |_data| {},
        ));
        AppServices {
            store: ReviewStore::new(vec!["/root".into()]),
            layer,
            service,
            capabilities: caps,
            ffmpeg: FfmpegInfo {
                ffmpeg: None,
                ffprobe: None,
                version: None,
            },
            roots_source: RootsSource::Cli,
            version: "0.0.0".into(),
            command: "astroshot review".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{FakeService, services};
    use super::*;
    use crate::terminal::probe::GraphicsProtocol;

    #[tokio::test]
    async fn use_services_returns_the_provided_services() {
        let services = services(GraphicsProtocol::None, FakeService::new(false));
        let found = use_services(Some(&services)).unwrap();
        assert_eq!(found.version, "0.0.0");
        assert_eq!(found.roots_source.as_str(), "cli");
    }

    #[test]
    fn use_services_errors_when_the_context_is_missing() {
        let error = use_services(None).err().unwrap();
        assert_eq!(error.to_string(), "ServicesContext is missing");
    }

    #[test]
    fn roots_source_spells_match_the_ts_literals() {
        let spelled: Vec<_> = [RootsSource::App, RootsSource::Cli, RootsSource::Cwd]
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(spelled, ["app", "cli", "cwd"]);
    }
}
