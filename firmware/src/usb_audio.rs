//! USB Audio Class 1.0 — device-to-host audio source (microphone).
//!
//! Implements a minimal UAC 1.0 streaming interface that sends mono 16-bit
//! PCM audio at 22 050 Hz from the synth engine to the USB host. The device
//! acts as a USB microphone.
//!
//! The design closely follows the existing `embassy_usb::class::uac1::speaker`
//! module but reverses the data flow direction:
//! - Input Terminal  = embedded synth (type: microphone)
//! - Output Terminal = USB streaming OUT to host
//! - Isochronous IN endpoint carries audio data
//! - Isochronous OUT endpoint carries feedback (optional, omitted for simplicity —
//!   we use an adaptive sync model so the host adapts to our rate)

use embassy_usb::control::{InResponse, OutResponse, Recipient, Request, RequestType};
use embassy_usb::descriptor::{SynchronizationType, UsageType};
use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointType};
use embassy_usb::types::InterfaceNumber;
use embassy_usb::{Builder, Handler};

use crate::synth::SAMPLE_RATE;

// -----------------------------------------------------------------------
// UAC 1.0 class codes (subset needed for our microphone)
// -----------------------------------------------------------------------

const USB_AUDIO_CLASS: u8 = 0x01;
const USB_AUDIOCONTROL_SUBCLASS: u8 = 0x01;
const USB_AUDIOSTREAMING_SUBCLASS: u8 = 0x02;
const PROTOCOL_NONE: u8 = 0x00;

const CS_INTERFACE: u8 = 0x24;
const CS_ENDPOINT: u8 = 0x25;

const HEADER_SUBTYPE: u8 = 0x01;
const INPUT_TERMINAL: u8 = 0x02;
const OUTPUT_TERMINAL: u8 = 0x03;
const FEATURE_UNIT: u8 = 0x06;

const AS_GENERAL: u8 = 0x01;
const FORMAT_TYPE: u8 = 0x02;
const FORMAT_TYPE_I: u8 = 0x01;
const PCM: u16 = 0x0001;

const ADC_VERSION: u16 = 0x0100;

// Terminal types
const USB_STREAMING: u16 = 0x0101;
const IN_MICROPHONE: u16 = 0x0201;

// Feature unit control selectors
const FU_CONTROL_UNDEFINED: u8 = 0x00;
const MUTE_CONTROL: u8 = 0x01;
const VOLUME_CONTROL: u8 = 0x02;

// Request codes
const SET_CUR: u8 = 0x01;
const GET_CUR: u8 = 0x81;
const GET_MIN: u8 = 0x82;
const GET_MAX: u8 = 0x83;
const GET_RES: u8 = 0x84;

// Endpoint control selectors
const SAMPLING_FREQ_CONTROL: u8 = 0x01;

// Unit IDs (must not collide with other UAC functions in the composite device)
const INPUT_UNIT_ID: u8 = 0x11;
const FEATURE_UNIT_ID: u8 = 0x12;
const OUTPUT_UNIT_ID: u8 = 0x13;

// Volume: 8q8 fixed-point dB. Range -100 dB to 0 dB, step 1 dB (256 in 8q8).
const VOLUME_STEPS_PER_DB: i16 = 256;
const MIN_VOLUME_DB: i16 = -100;
const MAX_VOLUME_DB: i16 = 0;

// -----------------------------------------------------------------------
// State shared between the control handler and the application
// -----------------------------------------------------------------------

/// Persistent state for the USB audio source. Must be `static`.
pub struct UacState<'d> {
    control: Option<UacControl<'d>>,
}

impl<'d> UacState<'d> {
    pub const fn new() -> Self {
        Self { control: None }
    }
}

// -----------------------------------------------------------------------
// Public API
// -----------------------------------------------------------------------

/// Build a UAC 1.0 microphone function and return the streaming endpoint writer.
///
/// # Arguments
/// * `builder`  – the composite USB device builder
/// * `state`    – static state storage
/// * `max_packet_size` – max bytes per isochronous IN packet.
///   At 22 050 Hz mono 16-bit that is ≈ 44 bytes/frame, but we use a
///   generous value to absorb jitter.
///
/// # Returns
/// An [`AudioStream`] that the application uses to write PCM packets.
pub fn build<'d, D: Driver<'d>>(
    builder: &mut Builder<'d, D>,
    state: &'d mut UacState<'d>,
    max_packet_size: u16,
) -> AudioStream<'d, D> {
    let mut func = builder.function(USB_AUDIO_CLASS, USB_AUDIOCONTROL_SUBCLASS, PROTOCOL_NONE);

    // ------------------------------------------------------------------
    // AudioControl interface (mandatory) [UAC 4.3.1]
    // ------------------------------------------------------------------
    let mut iface = func.interface();
    let control_iface_num = iface.interface_number();
    let streaming_iface_num = u8::from(control_iface_num) + 1;

    let mut alt = iface.alt_setting(
        USB_AUDIO_CLASS,
        USB_AUDIOCONTROL_SUBCLASS,
        PROTOCOL_NONE,
        None,
    );

    // Terminal topology:
    //   InputTerminal (microphone/synth) -> FeatureUnit (mute) -> OutputTerminal (USB streaming)

    // Input Terminal: embedded synth presented as a microphone
    let input_terminal = [
        INPUT_TERMINAL,             // bDescriptorSubtype
        INPUT_UNIT_ID,              // bTerminalID
        IN_MICROPHONE as u8,        // wTerminalType low
        (IN_MICROPHONE >> 8) as u8, // wTerminalType high
        0x00,                       // bAssocTerminal
        0x01,                       // bNrChannels (mono)
        0x00,                       // wChannelConfig low (mono = center)
        0x00,                       // wChannelConfig high
        0x00,                       // iChannelNames
        0x00,                       // iTerminal
    ];

    // Feature Unit: mute + volume on one channel
    let feature_unit = [
        FEATURE_UNIT,                  // bDescriptorSubtype
        FEATURE_UNIT_ID,               // bUnitID
        INPUT_UNIT_ID,                 // bSourceID
        0x01,                          // bControlSize (1 byte)
        FU_CONTROL_UNDEFINED,          // Master controls (none)
        MUTE_CONTROL | VOLUME_CONTROL, // Channel 1 controls
        0x00,                          // iFeature
    ];

    // Output Terminal: USB streaming to host
    let output_terminal = [
        OUTPUT_TERMINAL,            // bDescriptorSubtype
        OUTPUT_UNIT_ID,             // bTerminalID
        USB_STREAMING as u8,        // wTerminalType low
        (USB_STREAMING >> 8) as u8, // wTerminalType high
        0x00,                       // bAssocTerminal
        FEATURE_UNIT_ID,            // bSourceID
        0x00,                       // iTerminal
    ];

    // Class-specific AC interface header
    const HDR_SIZE: usize = 2; // descriptor type + length prefix added by embassy
    let total_len = (7 + HDR_SIZE)
        + (input_terminal.len() + HDR_SIZE)
        + (feature_unit.len() + HDR_SIZE)
        + (output_terminal.len() + HDR_SIZE);

    let ac_header = [
        HEADER_SUBTYPE,           // bDescriptorSubtype
        ADC_VERSION as u8,        // bcdADC low
        (ADC_VERSION >> 8) as u8, // bcdADC high
        total_len as u8,          // wTotalLength low
        (total_len >> 8) as u8,   // wTotalLength high
        0x01,                     // bInCollection
        streaming_iface_num,      // baInterfaceNr(1)
    ];

    alt.descriptor(CS_INTERFACE, &ac_header);
    alt.descriptor(CS_INTERFACE, &input_terminal);
    alt.descriptor(CS_INTERFACE, &feature_unit);
    alt.descriptor(CS_INTERFACE, &output_terminal);

    // ------------------------------------------------------------------
    // AudioStreaming interface — alt setting 0 (zero-bandwidth)
    // ------------------------------------------------------------------
    let mut stream_iface = func.interface();
    let _zb = stream_iface.alt_setting(
        USB_AUDIO_CLASS,
        USB_AUDIOSTREAMING_SUBCLASS,
        PROTOCOL_NONE,
        None,
    );

    // ------------------------------------------------------------------
    // AudioStreaming interface — alt setting 1 (operational)
    // ------------------------------------------------------------------
    let mut stream_alt = stream_iface.alt_setting(
        USB_AUDIO_CLASS,
        USB_AUDIOSTREAMING_SUBCLASS,
        PROTOCOL_NONE,
        None,
    );

    // AS General descriptor
    stream_alt.descriptor(
        CS_INTERFACE,
        &[
            AS_GENERAL,       // bDescriptorSubtype
            OUTPUT_UNIT_ID,   // bTerminalLink (our output terminal)
            0x00,             // bDelay
            PCM as u8,        // wFormatTag low
            (PCM >> 8) as u8, // wFormatTag high
        ],
    );

    // Format Type I descriptor (mono, 16-bit, one sample rate)
    let sr = SAMPLE_RATE;
    let format_type = [
        FORMAT_TYPE,               // bDescriptorSubtype
        FORMAT_TYPE_I,             // bFormatType
        0x01,                      // bNrChannels (mono)
        0x02,                      // bSubframeSize (2 bytes = 16 bit)
        16,                        // bBitResolution
        0x01,                      // bSamFreqType (1 discrete rate)
        (sr & 0xFF) as u8,         // tSamFreq[0] low
        ((sr >> 8) & 0xFF) as u8,  // tSamFreq[0] mid
        ((sr >> 16) & 0xFF) as u8, // tSamFreq[0] high
    ];
    stream_alt.descriptor(CS_INTERFACE, &format_type);

    // Isochronous IN endpoint (device → host audio data).
    // We use Adaptive sync: the host adapts to whatever rate we push.
    let iso_ep = stream_alt.alloc_endpoint_in(EndpointType::Isochronous, None, max_packet_size, 1);

    stream_alt.endpoint_descriptor(
        iso_ep.info(),
        SynchronizationType::Adaptive,
        UsageType::DataEndpoint,
        &[
            0x00, // bRefresh
            0x00, // bSynchAddress (no feedback endpoint)
        ],
    );

    // Class-specific isochronous audio data endpoint descriptor
    stream_alt.descriptor(
        CS_ENDPOINT,
        &[
            AS_GENERAL,            // bDescriptorSubtype
            SAMPLING_FREQ_CONTROL, // bmAttributes (sampling freq control)
            0x02,                  // bLockDelayUnits (PCM samples)
            0x00,                  // wLockDelay low
            0x00,                  // wLockDelay high
        ],
    );

    drop(func);

    // ------------------------------------------------------------------
    // Register the control request handler
    // ------------------------------------------------------------------
    let streaming_ep_addr: u8 = iso_ep.info().addr.into();
    state.control = Some(UacControl {
        control_iface: control_iface_num,
        streaming_ep_addr,
        muted: false,
        volume_8q8: MAX_VOLUME_DB * VOLUME_STEPS_PER_DB,
        _phantom: core::marker::PhantomData,
    });
    builder.handler(state.control.as_mut().unwrap());

    AudioStream { endpoint: iso_ep }
}

// -----------------------------------------------------------------------
// AudioStream — used by the application to push PCM packets
// -----------------------------------------------------------------------

/// Handle to the isochronous IN endpoint for sending audio data.
pub struct AudioStream<'d, D: Driver<'d>> {
    endpoint: D::EndpointIn,
}

impl<'d, D: Driver<'d>> AudioStream<'d, D> {
    /// Write one isochronous packet of PCM audio data.
    ///
    /// `data` must be an even number of bytes (16-bit LE samples).
    /// Returns `Err` if the endpoint is disabled (host hasn't selected
    /// alt-setting 1, or USB is disconnected).
    pub async fn write(&mut self, data: &[u8]) -> Result<(), EndpointError> {
        self.endpoint.write(data).await
    }

    /// Wait until the host activates the streaming interface (alt-setting 1).
    pub async fn wait_connection(&mut self) {
        self.endpoint.wait_enabled().await;
    }
}

// -----------------------------------------------------------------------
// Control request handler
// -----------------------------------------------------------------------

struct UacControl<'d> {
    control_iface: InterfaceNumber,
    streaming_ep_addr: u8,
    muted: bool,
    volume_8q8: i16,
    _phantom: core::marker::PhantomData<&'d ()>,
}

impl<'d> UacControl<'d> {
    fn interface_get<'r>(&self, req: Request, buf: &'r mut [u8]) -> Option<InResponse<'r>> {
        let iface = req.index as u8;
        let entity = (req.index >> 8) as u8;
        let channel = req.value as u8;
        let control_sel = (req.value >> 8) as u8;

        if iface != u8::from(self.control_iface) {
            return None;
        }
        if entity != FEATURE_UNIT_ID {
            return Some(InResponse::Rejected);
        }

        // Only channel 1 (our single mono channel) is valid, or channel 0 (master).
        if channel > 1 {
            return Some(InResponse::Rejected);
        }

        match req.request {
            GET_CUR => match control_sel {
                MUTE_CONTROL => {
                    buf[0] = u8::from(self.muted);
                    Some(InResponse::Accepted(&buf[..1]))
                }
                VOLUME_CONTROL => {
                    let v = self.volume_8q8;
                    buf[0] = v as u8;
                    buf[1] = (v >> 8) as u8;
                    Some(InResponse::Accepted(&buf[..2]))
                }
                _ => Some(InResponse::Rejected),
            },
            GET_MIN if control_sel == VOLUME_CONTROL => {
                let v = MIN_VOLUME_DB * VOLUME_STEPS_PER_DB;
                buf[0] = v as u8;
                buf[1] = (v >> 8) as u8;
                Some(InResponse::Accepted(&buf[..2]))
            }
            GET_MAX if control_sel == VOLUME_CONTROL => {
                let v = MAX_VOLUME_DB * VOLUME_STEPS_PER_DB;
                buf[0] = v as u8;
                buf[1] = (v >> 8) as u8;
                Some(InResponse::Accepted(&buf[..2]))
            }
            GET_RES if control_sel == VOLUME_CONTROL => {
                buf[0] = VOLUME_STEPS_PER_DB as u8;
                buf[1] = (VOLUME_STEPS_PER_DB >> 8) as u8;
                Some(InResponse::Accepted(&buf[..2]))
            }
            _ => Some(InResponse::Rejected),
        }
    }

    fn interface_set(&mut self, req: Request, data: &[u8]) -> Option<OutResponse> {
        let iface = req.index as u8;
        let entity = (req.index >> 8) as u8;
        let control_sel = (req.value >> 8) as u8;

        if iface != u8::from(self.control_iface) {
            return None;
        }
        if entity != FEATURE_UNIT_ID {
            return Some(OutResponse::Rejected);
        }
        if req.request != SET_CUR {
            return Some(OutResponse::Rejected);
        }

        match control_sel {
            MUTE_CONTROL => {
                if !data.is_empty() {
                    self.muted = data[0] != 0;
                }
                Some(OutResponse::Accepted)
            }
            VOLUME_CONTROL => {
                if data.len() >= 2 {
                    self.volume_8q8 = i16::from_le_bytes([data[0], data[1]]);
                }
                Some(OutResponse::Accepted)
            }
            _ => Some(OutResponse::Rejected),
        }
    }

    fn endpoint_get<'r>(&self, req: Request, buf: &'r mut [u8]) -> Option<InResponse<'r>> {
        let ep = req.index as u8;
        let control_sel = (req.value >> 8) as u8;

        if ep != self.streaming_ep_addr {
            return None;
        }
        if control_sel != SAMPLING_FREQ_CONTROL {
            return Some(InResponse::Rejected);
        }

        let sr = SAMPLE_RATE;
        buf[0] = (sr & 0xFF) as u8;
        buf[1] = ((sr >> 8) & 0xFF) as u8;
        buf[2] = ((sr >> 16) & 0xFF) as u8;
        Some(InResponse::Accepted(&buf[..3]))
    }

    fn endpoint_set(&mut self, req: Request, _data: &[u8]) -> Option<OutResponse> {
        let ep = req.index as u8;
        if ep != self.streaming_ep_addr {
            return None;
        }
        // We only support one fixed sample rate; accept but ignore SET_CUR.
        Some(OutResponse::Accepted)
    }
}

impl Handler for UacControl<'_> {
    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        if req.request_type != RequestType::Class {
            return None;
        }
        match req.recipient {
            Recipient::Interface => self.interface_get(req, buf),
            Recipient::Endpoint => self.endpoint_get(req, buf),
            _ => None,
        }
    }

    fn control_out(&mut self, req: Request, data: &[u8]) -> Option<OutResponse> {
        if req.request_type != RequestType::Class {
            return None;
        }
        match req.recipient {
            Recipient::Interface => self.interface_set(req, data),
            Recipient::Endpoint => self.endpoint_set(req, data),
            _ => None,
        }
    }
}
