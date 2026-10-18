//! The audio device: listing, choosing by name, and the cpal stream that runs the
//! [`Renderer`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, DeviceId, FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
};

use crate::error::PlaybackError;
use crate::ring::Renderer;
use crate::source::AudioFormat;

/// Frames per request on a PulseAudio stream at 48 kHz, scaled to the device's
/// rate: PipeWire's default quantum. The server holds two of them, about 43 ms.
/// cpal's `BufferSize::Default` leaves the size to the server, which buffers 2 s,
/// so pause, seek and volume would be heard 2 s late. A shorter period lowers the
/// quantum of the whole PipeWire graph, which wakes every app's audio more often.
const PULSE_PERIOD_AT_48K: u32 = 1024;

/// How often a PulseAudio stream checks that it still plays on the device `open`
/// would pick (see [`watch`]).
const WATCH_INTERVAL: Duration = Duration::from_millis(500);

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

/// The cpal host, shared by the device list and the output. On Linux it is
/// PulseAudio when a PulseAudio or pipewire-pulse server answers, ALSA otherwise.
/// A PulseAudio host holds a connection whose thread never ends once the host is
/// dropped, so one host serves the whole run. It is replaced when its connection
/// has died (the server restarted or stopped), and ALSA is replaced when a server
/// is there again.
fn host() -> Arc<cpal::Host> {
    static HOST: Mutex<Option<Arc<cpal::Host>>> = Mutex::new(None);
    let mut host = HOST.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(current) = host.as_ref()
        && still_usable(current)
    {
        return current.clone();
    }
    let fresh = Arc::new(cpal::default_host());
    *host = Some(fresh.clone());
    fresh
}

fn still_usable(host: &cpal::Host) -> bool {
    if is_pulse(host) {
        host.output_devices().is_ok()
    } else {
        // Only checks for the server's socket; connecting is `default_host`'s job.
        !cpal::available_hosts()
            .iter()
            .any(|id| id.name() == "PulseAudio")
    }
}

fn is_pulse(host: &cpal::Host) -> bool {
    host.id().name() == "PulseAudio"
}

/// Names of the output devices, for a settings list. The system default is not
/// listed by name; select it with `None`. With PulseAudio these are the server's
/// output names; with ALSA its utility plugins are left out (see
/// [`hidden_alsa_device`]), and a saved name still opens.
pub fn output_devices() -> Result<Vec<String>, PlaybackError> {
    let host = host();
    let alsa = host.id().name() == "ALSA";
    let mut names = Vec::new();
    for device in host.output_devices().map_err(output_error)? {
        let Ok(description) = device.description() else {
            continue;
        };
        // cpal gives the ALSA PCM name as the driver.
        if alsa && description.driver().is_some_and(hidden_alsa_device) {
            continue;
        }
        let name = description.name().to_owned();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Ok(names)
}

/// Whether an ALSA PCM is left out of the device list: the null device, the
/// sample rate converters, the up- and downmixers, the speex filter, the OSS
/// plugin (it needs the kernel's long-gone `/dev/dsp`), the USB stream plugin that
/// every USB card lists, and `default` and `sysdefault`, which "System default"
/// stands for. Sound servers (`pipewire`, `pulse`, `jack`) and the cards' own PCMs
/// (`sysdefault:CARD=...`, `front:...`, `hdmi:...`) stay.
fn hidden_alsa_device(pcm: &str) -> bool {
    const HIDDEN: [&str; 10] = [
        "null",
        "default",
        "sysdefault",
        "lavrate",
        "samplerate",
        "speexrate",
        "speex",
        "upmix",
        "vdownmix",
        "oss",
    ];
    match pcm.split_once(':') {
        Some((plugin, _card)) => plugin == "usbstream",
        None => HIDDEN.contains(&pcm),
    }
}

fn output_error(error: cpal::Error) -> PlaybackError {
    PlaybackError::Output(error.to_string())
}

fn device_name(device: &Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_owned())
}

/// The device picked by `open`, with what `start` needs.
struct Chosen {
    host: Arc<cpal::Host>,
    requested: Option<String>,
    device: Device,
    config: StreamConfig,
    sample_format: SampleFormat,
}

pub(crate) struct CpalOutput {
    chosen: Option<Chosen>,
    stream: Option<Stream>,
    /// Set when the running stream needs opening again. Each stream gets a new
    /// flag, so a late report about an old stream can't touch the next one.
    lost: Arc<AtomicBool>,
}

impl CpalOutput {
    pub(crate) fn new() -> Self {
        CpalOutput {
            chosen: None,
            stream: None,
            lost: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// The device `open` takes: the one named `requested` when it is there, the
/// system default otherwise (then `true`: the requested one was missing).
fn pick(host: &cpal::Host, requested: Option<&str>) -> Result<(Device, bool), PlaybackError> {
    let found = requested.and_then(|name| {
        host.output_devices()
            .ok()?
            .find(|device| device_name(device).as_deref() == Some(name))
    });
    let fell_back = requested.is_some() && found.is_none();
    let device = match found {
        Some(device) => device,
        None => host
            .default_output_device()
            .ok_or_else(|| PlaybackError::Output("no audio output device".into()))?,
    };
    Ok((device, fell_back))
}

/// Checks a PulseAudio stream's device from a thread of its own until the stream
/// ends (its `lost` flag, held weakly, is dropped). The server moves a stream by
/// itself when its device goes away, and cpal reports nothing; PulseAudio also
/// keeps a stream on its device when the system default changes. So every
/// [`WATCH_INTERVAL`] the device `open` would pick now is looked up, and when it
/// is another one (the chosen device went or came back, or the default changed),
/// the stream is marked lost and the engine opens the output again.
fn watch(
    host: Arc<cpal::Host>,
    requested: Option<String>,
    playing: DeviceId,
    lost: &Arc<AtomicBool>,
) {
    let lost = Arc::downgrade(lost);
    // Without the thread the stream still plays; it only stays where it is.
    let _ = thread::Builder::new()
        .name("vcmp-output-watch".into())
        .spawn(move || watch_loop(&host, requested.as_deref(), &playing, &lost));
}

fn watch_loop(
    host: &cpal::Host,
    requested: Option<&str>,
    playing: &DeviceId,
    lost: &Weak<AtomicBool>,
) {
    loop {
        thread::sleep(WATCH_INTERVAL);
        let Some(lost) = lost.upgrade() else {
            return;
        };
        // No device at all (or no server): the stream itself reports a dead
        // connection.
        let Ok((device, _)) = pick(host, requested) else {
            continue;
        };
        if device.id().ok().as_ref() != Some(playing) {
            lost.store(true, Ordering::Relaxed);
            return;
        }
    }
}

/// The device's default configuration, switched to stereo when the device offers
/// it at the same rate and sample format. A PulseAudio stream gets a fixed period
/// (see [`PULSE_PERIOD_AT_48K`]).
fn choose_config(
    device: &Device,
    pulse: bool,
) -> Result<(StreamConfig, SampleFormat), PlaybackError> {
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
    if pulse {
        config.buffer_size =
            BufferSize::Fixed((config.sample_rate * PULSE_PERIOD_AT_48K).div_ceil(48_000));
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
        let host = host();
        let (device, fell_back) = pick(&host, requested)?;
        let (config, sample_format) = choose_config(&device, is_pulse(&host))?;
        let name = device_name(&device).unwrap_or_else(|| "System default".into());
        let format = AudioFormat {
            sample_rate: config.sample_rate,
            channels: config.channels,
        };
        self.chosen = Some(Chosen {
            host,
            requested: requested.map(str::to_owned),
            device,
            config,
            sample_format,
        });
        Ok(OpenedDevice {
            name,
            format,
            fell_back,
        })
    }

    fn start(&mut self, renderer: Renderer) -> Result<(), PlaybackError> {
        let chosen = self
            .chosen
            .as_ref()
            .ok_or_else(|| PlaybackError::Output("no device chosen".into()))?;
        let lost = self.lost.clone();
        let (device, config) = (&chosen.device, chosen.config);
        let stream = match chosen.sample_format {
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
        if is_pulse(&chosen.host)
            && let Ok(playing) = device.id()
        {
            watch(
                chosen.host.clone(),
                chosen.requested.clone(),
                playing,
                &self.lost,
            );
        }
        self.stream = Some(stream);
        Ok(())
    }

    fn close(&mut self) {
        self.stream = None;
        self.chosen = None;
        self.lost = Arc::new(AtomicBool::new(false));
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

#[cfg(test)]
mod tests {
    use super::hidden_alsa_device;

    #[test]
    fn alsa_plugins_are_hidden_and_cards_and_servers_stay() {
        // What `aplay -L` and cpal list on a PipeWire desktop with a USB interface,
        // HDMI and on-board audio.
        let hidden = [
            "null",
            "default",
            "sysdefault",
            "lavrate",
            "samplerate",
            "speexrate",
            "speex",
            "upmix",
            "vdownmix",
            "oss",
            "usbstream:CARD=USB",
            "usbstream:CARD=HD3000",
        ];
        let shown = [
            "pipewire",
            "pulse",
            "jack",
            "sysdefault:CARD=USB",
            "front:CARD=USB,DEV=0",
            "iec958:CARD=Generic,DEV=0",
            "hdmi:CARD=HDMI,DEV=0",
            "hw:CARD=0,DEV=0",
            "plughw:CARD=0,DEV=0",
            "default:CARD=USB",
        ];
        for pcm in hidden {
            assert!(hidden_alsa_device(pcm), "{pcm} is hidden");
        }
        for pcm in shown {
            assert!(!hidden_alsa_device(pcm), "{pcm} is shown");
        }
    }
}
