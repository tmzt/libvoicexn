//! The default devices, with cpal (feature `cpal`), as voicexn-tty opens
//! them. Moved from flankr's `flankr-audio` 0.2.0.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::Format;
use crate::capture::Mic;
use crate::playback::Playback;

/// A default device and its default configuration.
pub(crate) struct Device {
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
}

/// The open streams; dropping them closes the devices.
pub(crate) struct Streams {
    pub(crate) _input: Option<cpal::Stream>,
    pub(crate) _output: cpal::Stream,
}

impl Device {
    /// The default input.
    pub(crate) fn input() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or("capture: no input device")?;
        let config = device
            .default_input_config()
            .map_err(|e| format!("capture: {e}"))?;
        log::info!(
            "session: capture {} Hz, {} channels",
            config.sample_rate(),
            config.channels()
        );
        Ok(Self { device, config })
    }

    /// The default output.
    pub(crate) fn output() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("playback: no output device")?;
        let config = device
            .default_output_config()
            .map_err(|e| format!("playback: {e}"))?;
        log::info!(
            "session: playback {} Hz, {} channels",
            config.sample_rate(),
            config.channels()
        );
        Ok(Self { device, config })
    }

    pub(crate) fn format(&self) -> Format {
        Format {
            rate: self.config.sample_rate(),
            channels: self.config.channels(),
        }
    }

    /// Starts the input with `mic` as its callback.
    pub(crate) fn capture(self, mut mic: Mic) -> Result<cpal::Stream, String> {
        let stream = self
            .device
            .build_input_stream::<f32, _, _>(
                self.config.config(),
                move |samples: &[f32], _: &cpal::InputCallbackInfo| mic.push(samples),
                |e| log::error!("capture: {e}"),
                None,
            )
            .map_err(|e| format!("capture: {e}"))?;
        stream.play().map_err(|e| format!("capture: {e}"))?;
        Ok(stream)
    }

    /// Starts the output with `playback` as its callback.
    pub(crate) fn play(self, mut playback: Playback) -> Result<cpal::Stream, String> {
        let stream = self
            .device
            .build_output_stream::<f32, _, _>(
                self.config.config(),
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| playback.fill(out),
                |e| log::error!("playback: {e}"),
                None,
            )
            .map_err(|e| format!("playback: {e}"))?;
        stream.play().map_err(|e| format!("playback: {e}"))?;
        Ok(stream)
    }
}
