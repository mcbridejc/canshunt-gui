mod client_worker;
mod protocol;
mod transport;
mod zencan_transport;

use client_worker::{ClientHandle, RawFrame};
use protocol::{Device, DeviceIdentity, PdoConfig};
use tokio::sync::Mutex;
use transport::InterfaceDescriptor;

struct AppState {
    client: Mutex<Option<ClientHandle>>,
}

#[tauri::command]
fn list_interfaces() -> Result<Vec<InterfaceDescriptor>, String> {
    transport::list_interfaces()
}

#[tauri::command]
async fn connect(
    state: tauri::State<'_, AppState>,
    descriptor: InterfaceDescriptor,
) -> Result<Option<Device>, String> {
    let direct_serial = if descriptor.kind == "usb" {
        Some(
            descriptor
                .serial
                .ok_or("The selected gs_usb device did not report a usable serial number")?,
        )
    } else {
        None
    };
    let bus = transport::open(&descriptor)?;
    let client = ClientHandle::start(bus)?;
    let direct_device = match direct_serial {
        Some(serial) => Some(client.direct_device(serial).await?),
        None => None,
    };
    *state.client.lock().await = Some(client);
    Ok(direct_device)
}

#[tauri::command]
async fn disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    *state.client.lock().await = None;
    Ok(())
}

#[tauri::command]
async fn scan_bus(state: tauri::State<'_, AppState>) -> Result<Vec<Device>, String> {
    let guard = state.client.lock().await;
    guard
        .as_ref()
        .ok_or("Not connected to a CAN interface")?
        .scan()
        .await
}

#[tauri::command]
async fn read_pdos(
    state: tauri::State<'_, AppState>,
    node_id: u8,
) -> Result<Vec<PdoConfig>, String> {
    let guard = state.client.lock().await;
    guard
        .as_ref()
        .ok_or("Not connected to a CAN interface")?
        .read_pdos(node_id)
        .await
}

#[tauri::command]
async fn write_pdos(
    state: tauri::State<'_, AppState>,
    node_id: u8,
    pdos: Vec<PdoConfig>,
) -> Result<(), String> {
    let guard = state.client.lock().await;
    guard
        .as_ref()
        .ok_or("Not connected to a CAN interface")?
        .write_pdos(node_id, pdos)
        .await
}

#[tauri::command]
async fn assign_node_id(
    state: tauri::State<'_, AppState>,
    identity: DeviceIdentity,
    new_node_id: u8,
) -> Result<(), String> {
    let guard = state.client.lock().await;
    guard
        .as_ref()
        .ok_or("Not connected to a CAN interface")?
        .assign(identity, new_node_id)
        .await
}

#[tauri::command]
async fn set_baudrate(
    state: tauri::State<'_, AppState>,
    node_id: u8,
    value: u8,
) -> Result<(), String> {
    let guard = state.client.lock().await;
    guard
        .as_ref()
        .ok_or("Not connected to a CAN interface")?
        .set_baud(node_id, value)
        .await
}

#[tauri::command]
async fn poll_frames(state: tauri::State<'_, AppState>) -> Result<Vec<RawFrame>, String> {
    let guard = state.client.lock().await;
    guard
        .as_ref()
        .ok_or("Not connected to a CAN interface")?
        .poll()
        .await
}

#[tauri::command]
async fn set_nmt_state(
    state: tauri::State<'_, AppState>,
    node_id: u8,
    nmt_state: String,
) -> Result<String, String> {
    let guard = state.client.lock().await;
    guard
        .as_ref()
        .ok_or("Not connected to a CAN interface")?
        .set_nmt(node_id, nmt_state)
        .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            client: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            list_interfaces,
            connect,
            disconnect,
            scan_bus,
            read_pdos,
            write_pdos,
            assign_node_id,
            set_baudrate,
            poll_frames,
            set_nmt_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running CANShunt");
}
