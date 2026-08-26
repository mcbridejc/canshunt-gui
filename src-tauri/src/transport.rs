use crate::protocol::{CanBus, CanFrame};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const VID: u16 = 0x1209;
const PID: u16 = 0x5F4D;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceDescriptor {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub serial: Option<u32>,
}

pub fn list_interfaces() -> Result<Vec<InterfaceDescriptor>, String> {
    let mut result = Vec::new();
    #[cfg(target_os = "linux")]
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_can = std::fs::read_to_string(path.join("type"))
                .map(|v| v.trim() == "280")
                .unwrap_or(false);
            if is_can {
                let name = entry.file_name().to_string_lossy().into_owned();
                result.push(InterfaceDescriptor {
                    kind: "socketcan".into(),
                    id: name.clone(),
                    label: name,
                    serial: None,
                });
            }
        }
    }
    let devices = rusb::devices().map_err(|e| format!("Could not enumerate USB devices: {e}"))?;
    for device in devices.iter() {
        let descriptor = match device.device_descriptor() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if descriptor.vendor_id() != VID || descriptor.product_id() != PID {
            continue;
        }
        let serial = device
            .open()
            .ok()
            .and_then(|h| h.read_serial_number_string_ascii(&descriptor).ok());
        let id = format!("{}:{}", device.bus_number(), device.address());
        let numeric_serial = serial.as_deref().and_then(parse_usb_serial);
        let label = serial
            .map(|s| format!("CANShunt {s}"))
            .unwrap_or_else(|| format!("CANShunt USB ({id})"));
        result.push(InterfaceDescriptor {
            kind: "usb".into(),
            id,
            label,
            serial: numeric_serial,
        });
    }
    Ok(result)
}

fn parse_usb_serial(value: &str) -> Option<u32> {
    if value.len() != 8 {
        return None;
    }
    u32::from_str_radix(value, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::parse_usb_serial;

    #[test]
    fn parses_firmware_usb_serial_as_lss_identity_value() {
        assert_eq!(parse_usb_serial("1234ABCD"), Some(0x1234_ABCD));
        assert_eq!(parse_usb_serial("00000001"), Some(1));
        assert_eq!(parse_usb_serial("not-hex!"), None);
    }
}

pub fn open(descriptor: &InterfaceDescriptor) -> Result<Box<dyn CanBus>, String> {
    match descriptor.kind.as_str() {
        #[cfg(target_os = "linux")]
        "socketcan" => Ok(Box::new(SocketCanBus::open(&descriptor.id)?)),
        #[cfg(not(target_os = "linux"))]
        "socketcan" => Err("SocketCAN is available on Linux only".into()),
        "usb" => Ok(Box::new(GsUsbBus::open(&descriptor.id)?)),
        _ => Err("Unknown CAN interface type".into()),
    }
}

#[cfg(target_os = "linux")]
struct SocketCanBus(socketcan::CanSocket);

#[cfg(target_os = "linux")]
impl SocketCanBus {
    fn open(name: &str) -> Result<Self, String> {
        use socketcan::Socket;
        let socket =
            socketcan::CanSocket::open(name).map_err(|e| format!("Could not open {name}: {e}"))?;
        socket
            .set_read_timeout(Duration::from_millis(20))
            .map_err(|e| e.to_string())?;
        Ok(Self(socket))
    }
}

#[cfg(target_os = "linux")]
impl CanBus for SocketCanBus {
    fn send(&mut self, frame: &CanFrame) -> Result<(), String> {
        use socketcan::{CanFrame as SocketFrame, EmbeddedFrame, ExtendedId, Socket, StandardId};
        let native = if frame.extended {
            SocketFrame::new(
                ExtendedId::new(frame.id).ok_or("Invalid extended CAN ID")?,
                &frame.data,
            )
        } else {
            SocketFrame::new(
                StandardId::new(frame.id as u16).ok_or("Invalid standard CAN ID")?,
                &frame.data,
            )
        }
        .ok_or("Invalid CAN frame")?;
        self.0
            .write_frame(&native)
            .map_err(|e| format!("CAN transmit failed: {e}"))
    }
    fn receive(&mut self, timeout: Duration) -> Result<Option<CanFrame>, String> {
        use socketcan::{EmbeddedFrame, Id, Socket};
        self.0
            .set_read_timeout(timeout)
            .map_err(|e| e.to_string())?;
        match self.0.read_frame() {
            Ok(frame) => {
                let (id, extended) = match frame.id() {
                    Id::Standard(v) => (v.as_raw() as u32, false),
                    Id::Extended(v) => (v.as_raw(), true),
                };
                Ok(Some(CanFrame {
                    id,
                    extended,
                    data: frame.data().to_vec(),
                }))
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(format!("CAN receive failed: {e}")),
        }
    }
}

struct GsUsbBus {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
    interface: u8,
    endpoint_in: u8,
    endpoint_out: u8,
    echo: u32,
    reattach: bool,
}

impl GsUsbBus {
    fn open(id: &str) -> Result<Self, String> {
        let (bus, address) = id.split_once(':').ok_or("Invalid USB device identifier")?;
        let bus: u8 = bus.parse().map_err(|_| "Invalid USB bus")?;
        let address: u8 = address.parse().map_err(|_| "Invalid USB address")?;
        let devices = rusb::devices().map_err(|e| e.to_string())?;
        let device = devices
            .iter()
            .find(|d| d.bus_number() == bus && d.address() == address)
            .ok_or("USB device is no longer present")?;
        let handle = device
            .open()
            .map_err(|e| format!("Could not open USB device: {e}"))?;
        let config = device
            .active_config_descriptor()
            .or_else(|_| device.config_descriptor(0))
            .map_err(|e| e.to_string())?;
        let mut selected = None;
        for interface in config.interfaces() {
            for descriptor in interface.descriptors() {
                if descriptor.class_code() != 0xFF {
                    continue;
                }
                let mut input = None;
                let mut output = None;
                for endpoint in descriptor.endpoint_descriptors() {
                    if endpoint.transfer_type() != rusb::TransferType::Bulk {
                        continue;
                    }
                    match endpoint.direction() {
                        rusb::Direction::In => input = Some(endpoint.address()),
                        rusb::Direction::Out => output = Some(endpoint.address()),
                    }
                }
                if let (Some(endpoint_in), Some(endpoint_out)) = (input, output) {
                    selected = Some((descriptor.interface_number(), endpoint_in, endpoint_out));
                    break;
                }
            }
        }
        let (interface, endpoint_in, endpoint_out) =
            selected.ok_or("gs_usb bulk endpoints were not found")?;
        let reattach = handle.kernel_driver_active(interface).unwrap_or(false);
        if reattach {
            handle
                .detach_kernel_driver(interface)
                .map_err(|e| format!("Could not detach kernel driver: {e}"))?;
        }
        handle
            .claim_interface(interface)
            .map_err(|e| format!("Could not claim USB interface: {e}"))?;
        handle
            .write_control(
                0x41,
                0,
                0,
                interface as u16,
                &0x0000_BEEFu32.to_le_bytes(),
                Duration::from_secs(1),
            )
            .map_err(|e| format!("gs_usb host setup failed: {e}"))?;
        let mut mode = Vec::from(1u32.to_le_bytes());
        mode.extend(0u32.to_le_bytes());
        handle
            .write_control(0x41, 2, 0, interface as u16, &mode, Duration::from_secs(1))
            .map_err(|e| format!("gs_usb start failed: {e}"))?;
        Ok(Self {
            handle,
            interface,
            endpoint_in,
            endpoint_out,
            echo: 0,
            reattach,
        })
    }
}

impl CanBus for GsUsbBus {
    fn send(&mut self, frame: &CanFrame) -> Result<(), String> {
        if frame.data.len() > 8 {
            return Err("Classic CAN payload exceeds 8 bytes".into());
        }
        let mut bytes = [0u8; 20];
        bytes[0..4].copy_from_slice(&self.echo.to_le_bytes());
        let id = frame.id | if frame.extended { 0x8000_0000 } else { 0 };
        bytes[4..8].copy_from_slice(&id.to_le_bytes());
        bytes[8] = frame.data.len() as u8;
        bytes[9] = 0;
        bytes[12..12 + frame.data.len()].copy_from_slice(&frame.data);
        self.echo = self.echo.wrapping_add(1);
        let written = self
            .handle
            .write_bulk(self.endpoint_out, &bytes, Duration::from_millis(250))
            .map_err(|e| format!("USB CAN transmit failed: {e}"))?;
        if written == bytes.len() {
            Ok(())
        } else {
            Err("Short USB CAN write".into())
        }
    }
    fn receive(&mut self, timeout: Duration) -> Result<Option<CanFrame>, String> {
        let mut bytes = [0u8; 64];
        match self.handle.read_bulk(self.endpoint_in, &mut bytes, timeout) {
            Ok(count) if count >= 20 => {
                let raw = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
                let len = usize::from(bytes[8].min(8));
                if raw & 0x2000_0000 != 0 {
                    return Ok(None);
                }
                Ok(Some(CanFrame {
                    id: raw & 0x1FFF_FFFF,
                    extended: raw & 0x8000_0000 != 0,
                    data: bytes[12..12 + len].to_vec(),
                }))
            }
            Ok(_) => Ok(None),
            Err(rusb::Error::Timeout) => Ok(None),
            Err(e) => Err(format!("USB CAN receive failed: {e}")),
        }
    }
}

impl Drop for GsUsbBus {
    fn drop(&mut self) {
        let mut mode = Vec::from(0u32.to_le_bytes());
        mode.extend(0u32.to_le_bytes());
        let _ = self.handle.write_control(
            0x41,
            2,
            0,
            self.interface as u16,
            &mode,
            Duration::from_millis(100),
        );
        let _ = self.handle.release_interface(self.interface);
        if self.reattach {
            let _ = self.handle.attach_kernel_driver(self.interface);
        }
    }
}
