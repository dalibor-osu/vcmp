//! The audio device: listing, choosing by name, and the cpal stream that runs the
//! [`Renderer`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig};

use crate::error::PlaybackError;
use crate::ring::Renderer;
use crate::source::AudioFormat;

/// A device that is ready to start.
pub(crate) struct OpenedDevice {
    pub(crate) name: String,
    pub(crate) format: AudioFormat,
    /// The requested device was missing and the system default was used instead.
    pub(crate) fell_back: bool,
}

/// Where the engine sends its audio. The engine thread owns it.
pub(crate) trait Output {
    /// Picks the device named `device` (or the system default) and its format.
    fn open(&mut self, device: Option<&str>) -> Result<OpenedDevice, PlaybackError>;
    /// Starts the stream of the device picked by the last `open`.
    fn start(&mut self, renderer: Renderer) -> Result<(), PlaybackError>;
    /// Stops and releases the stream.
    fn close(&mut self);
    /// Whether the running stream died (device unplugged, format changed) and needs
    /// to be opened again.
    fn lost(&self) -> bool;
}

/// Names of the output devices, for a settings list. The system default is not
/// listed by name; select it with `None`.
pub fn output_devices() -> Result<Vec<String>, PlaybackError> {
    let host = cpal::default_host();
    let mut names = Vec::new();
    for device in host.output_devices().map_err(output_error)? {
        if let Some(name) = device_name(&device)
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    Ok(names)
}

fn output_error(error: cpal::Error) -> PlaybackError {
    PlaybackError::Output(error.to_string())
}

fn device_name(device: &Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_owned())
}

pub(crate) struct CpalOutput {
    host: cpal::Host,
    chosen: Option<(Device, StreamConfig, SampleFormat)>,
    stream: Option<Stream>,
    lost: Arc<AtomicBool>,
}

impl CpalOutput {
    pub(crate) fn new() -> Self {
        CpalOutput {
            host: cpal::default_host(),
            chosen: None,
            stream: None,
            lost: Arc::new(AtomicBool::new(false)),
        }
    }

    fn find(&self, name: &str) -> Option<Device> {
        self.host
            .output_devices()
            .ok()?
            .find(|device| device_name(device).as_deref() == Some(name))
    }
}

/// The device's default configuration, switched to stereo when the device offers
/// it at the same rate and sample format.
fn choose_config(device: &Device) -> Result<(StreamConfig, SampleFormat), PlaybackError> {
    let default = device.default_output_config().map_err(output_error)?;
    let mut config = default.config();
    if config.channels != 2
        && let Ok(ranges) = device.supported_output_configs()
        && let Some(stereo) = ranges
            .filter(|range| {
                range.channels() == 2 && range.sample_format() == default.sample_format()
            })
            .find_map(|range| range.try_with_sample_rate(config.sample_rate))
    {
        config = stereo.config();
    }
    Ok((config, default.sample_format()))
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &Device,
    config: StreamConfig,
    mut renderer: Renderer,
    lost: Arc<AtomicBool>,
) -> Result<Stream, PlaybackError> {
    device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| renderer.render(data),
            move |error: cpal::Error| {
                if matches!(
                    error.kind(),
                    cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated
                ) {
                    lost.store(true, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(output_error)
}

impl Output for CpalOutput {
    fn open(&mut self, requested: Option<&str>) -> Result<OpenedDevice, PlaybackError> {
        self.close();
        let found = requested.and_then(|name| self.find(name));
        let fell_back = requested.is_some() && found.is_none();
        let device = match found {
            Some(device) => device,
            None => self
                .host
                .default_output_device()
                .ok_or_else(|| PlaybackError::Output("no audio output device".into()))?,
        };
        let (config, sample_format) = choose_config(&device)?;
        let name = device_name(&device).unwrap_or_else(|| "System default".into());
        let format = AudioFormat {
            sample_rate: config.sample_rate,
            channels: config.channels,
        };
        self.chosen = Some((device, config, sample_format));
        Ok(OpenedDevice {
            name,
            format,
            fell_back,
        })
    }

    fn start(&mut self, renderer: Renderer) -> Result<(), PlaybackError> {
        let (device, config, sample_format) = self
            .chosen
            .as_ref()
            .ok_or_else(|| PlaybackError::Output("no device chosen".into()))?;
        self.lost.store(false, Ordering::Relaxed);
        let lost = self.lost.clone();
        let config = *config;
        let stream = match sample_format {
            SampleFormat::F32 => build::<f32>(device, config, renderer, lost),
            SampleFormat::F64 => build::<f64>(device, config, renderer, lost),
            SampleFormat::I16 => build::<i16>(device, config, renderer, lost),
            SampleFormat::I24 => build::<cpal::I24>(device, config, renderer, lost),
            SampleFormat::I32 => build::<i32>(device, config, renderer, lost),
            SampleFormat::I8 => build::<i8>(device, config, renderer, lost),
            SampleFormat::U8 => build::<u8>(device, config, renderer, lost),
            SampleFormat::U16 => build::<u16>(device, config, renderer, lost),
            SampleFormat::U24 => build::<cpal::U24>(device, config, renderer, lost),
            SampleFormat::U32 => build::<u32>(device, config, renderer, lost),
            other => Err(PlaybackError::Output(format!(
                "unsupported sample format {other}"
            ))),
        }?;
        stream.play().map_err(output_error)?;
        self.stream = Some(stream);
        Ok(())
    }

    fn close(&mut self) {
        self.stream = None;
        self.chosen = None;
    }

    fn lost(&self) -> bool {
        self.stream.is_some() && self.lost.load(Ordering::Relaxed)
    }
}

/// A device for tests: the test thread pulls audio out of it in place of a sound
/// card, so playback can be checked sample by sample.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone)]
    pub(crate) struct TestDevice(Arc<Inner>);

    struct Inner {
        format: AudioFormat,
        devices: Vec<String>,
        renderer: Mutex<Option<Renderer>>,
        lost: AtomicBool,
        broken: AtomicBool,
    }

    pub(crate) const DEFAULT_NAME: &str = "Test default";

    impl TestDevice {
        pub(crate) fn new(format: AudioFormat, devices: &[&str]) -> Self {
            TestDevice(Arc::new(Inner {
                format,
                devices: devices.iter().map(|d| d.to_string()).collect(),
                renderer: Mutex::new(None),
                lost: AtomicBool::new(false),
                broken: AtomicBool::new(false),
            }))
        }

        pub(crate) fn output(&self) -> Box<dyn Output> {
            Box::new(TestOutput(self.clone()))
        }

        /// Renders into `out`; false while no stream is running.
        pub(crate) fn render(&self, out: &mut [f32]) -> bool {
            match self.0.renderer.lock().unwrap().as_mut() {
                Some(renderer) => {
                    renderer.render(out);
                    true
                }
                None => false,
            }
        }

        /// Frames ready to play.
        pub(crate) fn available(&self) -> usize {
            self.0
                .renderer
                .lock()
                .unwrap()
                .as_ref()
                .map_or(0, |r| r.available())
        }

        /// Frames the stream has taken so far.
        pub(crate) fn taken(&self) -> u64 {
            self.0
                .renderer
                .lock()
                .unwrap()
                .as_ref()
                .map_or(0, |r| r.taken())
        }

        /// Behaves as if the device were unplugged.
        pub(crate) fn unplug(&self) {
            self.0.lost.store(true, Ordering::Relaxed);
        }

        /// Makes every attempt to open a device fail.
        pub(crate) fn break_down(&self, broken: bool) {
            self.0.broken.store(broken, Ordering::Relaxed);
        }
    }

    struct TestOutput(TestDevice);

    impl Output for TestOutput {
        fn open(&mut self, device: Option<&str>) -> Result<OpenedDevice, PlaybackError> {
            self.close();
            let inner = &self.0.0;
            if inner.broken.load(Ordering::Relaxed) {
                return Err(PlaybackError::Output("test device unavailable".into()));
            }
            let found = device.filter(|name| inner.devices.iter().any(|d| d == name));
            Ok(OpenedDevice {
                name: found.unwrap_or(DEFAULT_NAME).to_owned(),
                format: inner.format,
                fell_back: device.is_some() && found.is_none(),
            })
        }

        fn start(&mut self, renderer: Renderer) -> Result<(), PlaybackError> {
            self.0.0.lost.store(false, Ordering::Relaxed);
            *self.0.0.renderer.lock().unwrap() = Some(renderer);
            Ok(())
        }

        fn close(&mut self) {
            *self.0.0.renderer.lock().unwrap() = None;
        }

        fn lost(&self) -> bool {
            self.0.0.lost.load(Ordering::Relaxed)
        }
    }
}
