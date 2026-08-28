use crate::zencan_transport::TransportSender;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use zencan_client::{
    BusManager,
    common::protocol::{ConfiguredNodeId, LssIdentity, LssState, NodeId},
};

pub const CANSHUNT_VENDOR: u32 = 0xCAFE;
pub const CANSHUNT_PRODUCT: u32 = 1050;

#[derive(Debug, Clone)]
pub struct CanFrame {
    pub id: u32,
    pub extended: bool,
    pub data: Vec<u8>,
}

pub trait CanBus: Send {
    fn send(&mut self, frame: &CanFrame) -> Result<(), String>;
    fn receive(&mut self, timeout: Duration) -> Result<Option<CanFrame>, String>;
    fn set_physical_can_enabled(&mut self, _enabled: bool) -> Result<bool, String> {
        Err("Physical CAN routing is available only for a direct CANShunt USB connection".into())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub node_id: u8,
    pub vendor_id: Option<u32>,
    pub product_code: Option<u32>,
    pub revision: Option<u32>,
    pub serial: Option<u32>,
    pub is_canshunt: bool,
    pub nmt_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub vendor_id: u32,
    pub product_code: u32,
    pub revision: u32,
    pub serial: u32,
}

impl From<DeviceIdentity> for LssIdentity {
    fn from(value: DeviceIdentity) -> Self {
        LssIdentity::new(
            value.vendor_id,
            value.product_code,
            value.revision,
            value.serial,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdoConfig {
    pub name: String,
    pub enabled: bool,
    pub can_id: u32,
    pub extended: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceConfig {
    pub pdos: Vec<PdoConfig>,
    pub baudrate: u8,
}

pub async fn scan(manager: &mut BusManager<TransportSender>) -> Result<Vec<Device>, String> {
    let mut devices = manager
        .lss_fastscan(Duration::from_millis(20))
        .await
        .into_iter()
        .map(|identity| Device {
            node_id: 0xFF,
            vendor_id: Some(identity.vendor_id),
            product_code: Some(identity.product_code),
            revision: Some(identity.revision),
            serial: Some(identity.serial),
            is_canshunt: identity.vendor_id == CANSHUNT_VENDOR
                && identity.product_code == CANSHUNT_PRODUCT,
            nmt_state: None,
        })
        .collect::<Vec<_>>();
    let configured = manager
        .scan_nodes()
        .await
        .map_err(|error| error.to_string())?;
    devices.extend(configured.into_iter().map(|node| {
        let identity = node.identity;
        Device {
            node_id: node.node_id,
            vendor_id: identity.map(|id| id.vendor_id),
            product_code: identity.map(|id| id.product_code),
            revision: identity.map(|id| id.revision),
            serial: identity.map(|id| id.serial),
            is_canshunt: identity.is_some_and(|id| {
                id.vendor_id == CANSHUNT_VENDOR && id.product_code == CANSHUNT_PRODUCT
            }),
            nmt_state: node.nmt_state.map(|state| state.to_string()),
        }
    }));
    Ok(devices)
}

pub async fn read_pdos(
    manager: &mut BusManager<TransportSender>,
    node: u8,
) -> Result<Vec<PdoConfig>, String> {
    const NAMES: [&str; 4] = ["Current low", "Current high", "Voltage low", "Voltage high"];
    let result = manager
        .read_pdo_config(configured_node(node)?)
        .await
        .map_err(|error| error.to_string())?;
    if result.tpdos.len() < 4 {
        return Err(format!("Device exposes only {} TPDOs", result.tpdos.len()));
    }
    Ok(result
        .tpdos
        .iter()
        .take(4)
        .enumerate()
        .map(|(index, pdo)| PdoConfig {
            name: NAMES[index].into(),
            enabled: pdo.enabled,
            can_id: pdo.cob_id.raw(),
            extended: pdo.cob_id.is_extended(),
        })
        .collect())
}

pub async fn read_device_config(
    manager: &mut BusManager<TransportSender>,
    node: u8,
) -> Result<DeviceConfig, String> {
    let pdos = read_pdos(manager, node).await?;
    let baudrate = manager
        .sdo_client(configured_node(node)?.raw())
        .read_u8(0x2200, 0)
        .await
        .map_err(|error| error.to_string())?;
    if baudrate > 6 {
        return Err(format!(
            "Device reported unsupported baud rate value {baudrate}"
        ));
    }
    Ok(DeviceConfig { pdos, baudrate })
}

pub async fn write_pdos(
    manager: &mut BusManager<TransportSender>,
    node: u8,
    pdos: &[PdoConfig],
) -> Result<(), String> {
    if pdos.len() != 4 {
        return Err("Exactly four TPDO configurations are required".into());
    }
    let node = configured_node(node)?;
    require_preoperational(manager, node.raw()).await?;
    let mut client = manager.sdo_client(node.raw());
    for (index, pdo) in pdos.iter().enumerate() {
        let max = if pdo.extended { 0x1FFF_FFFF } else { 0x7FF };
        if pdo.can_id > max {
            return Err(format!("{} CAN ID is out of range", pdo.name));
        }
        let comm = 0x1800 + index as u16;
        let mapping = 0x1A00 + index as u16;
        let frame_flag = if pdo.extended { 1 << 29 } else { 0 };
        client
            .write_u32(comm, 1, pdo.can_id | frame_flag | (1 << 31))
            .await
            .map_err(|error| error.to_string())?;
        client
            .write_u8(mapping, 0, 0)
            .await
            .map_err(|error| error.to_string())?;
        let object = if index < 2 { 0x2010u32 } else { 0x2011u32 };
        let first_sub = if index % 2 == 0 { 1 } else { 5 };
        for slot in 0..4u8 {
            let entry = (object << 16) | (((first_sub + slot) as u32) << 8) | 16;
            client
                .write_u32(mapping, slot + 1, entry)
                .await
                .map_err(|error| error.to_string())?;
        }
        client
            .write_u8(mapping, 0, 4)
            .await
            .map_err(|error| error.to_string())?;
        let disabled = if pdo.enabled { 0 } else { 1 << 31 };
        client
            .write_u32(comm, 1, pdo.can_id | frame_flag | disabled)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub async fn assign_node_id(
    manager: &mut BusManager<TransportSender>,
    identity: DeviceIdentity,
    new_node: u8,
) -> Result<(), String> {
    let configured = configured_node(new_node)?;
    let lss_identity = LssIdentity::from(identity);
    let current = manager
        .node_list()
        .await
        .into_iter()
        .find(|node| node.identity == Some(lss_identity))
        .ok_or("The device must have a configured node ID and be PreOperational")?;
    require_preoperational(manager, current.node_id).await?;
    manager
        .lss_activate(lss_identity)
        .await
        .map_err(|error| error.to_string())?;
    manager
        .lss_set_node_id(NodeId::Configured(configured))
        .await
        .map_err(|error| error.to_string())?;
    manager
        .lss_store_config()
        .await
        .map_err(|error| error.to_string())?;
    manager.lss_set_global_mode(LssState::Waiting).await;
    manager.nmt_reset_comms(new_node).await;
    Ok(())
}

pub async fn set_baudrate(
    manager: &mut BusManager<TransportSender>,
    node: u8,
    value: u8,
) -> Result<(), String> {
    if value > 6 {
        return Err("Invalid baud rate selection".into());
    }
    let node = configured_node(node)?;
    let mut client = manager.sdo_client(node.raw());
    client
        .write_u8(0x2200, 0, value)
        .await
        .map_err(|error| error.to_string())?;
    client
        .save_objects()
        .await
        .map_err(|error| format!("Baud rate was written but could not be persisted: {error}"))
}

pub async fn identify_device(
    manager: &mut BusManager<TransportSender>,
    node: u8,
) -> Result<(), String> {
    let node = configured_node(node)?;
    manager
        .sdo_client(node.raw())
        .write_u8(0x2F80, 0, 1)
        .await
        .map_err(|error| error.to_string())
}

fn configured_node(value: u8) -> Result<ConfiguredNodeId, String> {
    if !(1..=127).contains(&value) {
        return Err("Node ID must be between 1 and 127".into());
    }
    ConfiguredNodeId::new(value).map_err(|error| error.to_string())
}

async fn require_preoperational(
    manager: &BusManager<TransportSender>,
    node_id: u8,
) -> Result<(), String> {
    let state = manager
        .node_list()
        .await
        .into_iter()
        .find(|node| node.node_id == node_id)
        .and_then(|node| node.nmt_state);
    if state == Some(zencan_client::common::protocol::NmtState::PreOperational) {
        Ok(())
    } else {
        Err("Device must be in the PreOperational state".into())
    }
}
