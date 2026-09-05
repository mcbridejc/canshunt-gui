use crate::{
    protocol::{self, Device, DeviceConfig, DeviceIdentity, PdoConfig},
    zencan_transport::{self, TransportSender},
};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use zencan_client::{
    BusManager,
    common::{
        lss::{LssIdentity, LssRequest, LssResponse, LssState},
        messages::{NmtCommand, NmtCommandSpecifier},
        nmt::NmtState,
        traits::{AsyncCanReceiver, AsyncCanSender, CanSendError},
    },
};

#[derive(Debug, Serialize)]
pub struct RawFrame {
    pub id: u32,
    pub extended: bool,
    pub data: Vec<u8>,
}

enum Command {
    Scan(oneshot::Sender<Result<Vec<Device>, String>>),
    ReadConfig(u8, oneshot::Sender<Result<DeviceConfig, String>>),
    WritePdos(u8, Vec<PdoConfig>, oneshot::Sender<Result<(), String>>),
    Assign(DeviceIdentity, u8, oneshot::Sender<Result<(), String>>),
    SetBaud(u8, u8, oneshot::Sender<Result<(), String>>),
    Identify(u8, oneshot::Sender<Result<(), String>>),
    Poll(oneshot::Sender<Result<Vec<RawFrame>, String>>),
    SetNmt(u8, String, oneshot::Sender<Result<String, String>>),
    DirectDevice(u32, oneshot::Sender<Result<Device, String>>),
    SetPhysicalCan(bool, oneshot::Sender<Result<bool, String>>),
}

pub struct ClientHandle {
    commands: mpsc::UnboundedSender<Command>,
}

impl ClientHandle {
    pub fn start(bus: Box<dyn protocol::CanBus>) -> Result<Self, String> {
        let (commands, mut command_rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("zencan-client".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("zencan runtime");
                runtime.block_on(async move {
                    let (sender, receiver, mut discovery, mut monitor) =
                        zencan_transport::start(bus);
                    let mut control_sender = sender.clone();
                    let mut manager: BusManager<TransportSender> =
                        BusManager::new(sender, receiver);
                    while let Some(command) = command_rx.recv().await {
                        match command {
                            Command::Scan(reply) => {
                                let _ = reply.send(protocol::scan(&mut manager).await);
                            }
                            Command::ReadConfig(node, reply) => {
                                let _ = reply
                                    .send(protocol::read_device_config(&mut manager, node).await);
                            }
                            Command::WritePdos(node, pdos, reply) => {
                                let _ = reply
                                    .send(protocol::write_pdos(&mut manager, node, &pdos).await);
                            }
                            Command::Assign(identity, node, reply) => {
                                let _ = reply.send(
                                    protocol::assign_node_id(&mut manager, identity, node).await,
                                );
                            }
                            Command::SetBaud(node, value, reply) => {
                                let _ = reply
                                    .send(protocol::set_baudrate(&mut manager, node, value).await);
                            }
                            Command::Identify(node, reply) => {
                                let _ =
                                    reply.send(protocol::identify_device(&mut manager, node).await);
                            }
                            Command::Poll(reply) => {
                                let mut frames = Vec::new();
                                while frames.len() < 64 {
                                    let Some(message) = monitor.try_recv() else {
                                        break;
                                    };
                                    frames.push(RawFrame {
                                        id: message.id().raw(),
                                        extended: message.id().is_extended(),
                                        data: message.data().to_vec(),
                                    });
                                }
                                let _ = reply.send(Ok(frames));
                            }
                            Command::SetNmt(node, requested, reply) => {
                                let result = async {
                                    if !(1..=127).contains(&node) {
                                        return Err("NMT commands require a configured node".into());
                                    }
                                    let (command, expected) = match requested.as_str() {
                                        "Start" => {
                                            (NmtCommandSpecifier::Start, NmtState::Operational)
                                        }
                                        "ResetApp" => (
                                            NmtCommandSpecifier::ResetApp,
                                            NmtState::PreOperational,
                                        ),
                                        _ => return Err("Unsupported NMT action".into()),
                                    };
                                    control_sender
                                        .send(NmtCommand { cs: command, node }.into())
                                        .await
                                        .map_err(|error| error.message())?;
                                    tokio::time::sleep(std::time::Duration::from_millis(2000))
                                        .await;
                                    let observed = manager
                                        .node_list()
                                        .await
                                        .into_iter()
                                        .find(|entry| entry.node_id == node)
                                        .and_then(|entry| entry.nmt_state);
                                    if observed == Some(expected) {
                                        Ok(expected.to_string())
                                    } else {
                                        Err(format!(
                                            "Device did not report the requested {expected} state"
                                        ))
                                    }
                                }
                                .await;
                                let _ = reply.send(result);
                            }
                            Command::DirectDevice(serial, reply) => {
                                let identity = LssIdentity::new(
                                    protocol::CANSHUNT_VENDOR,
                                    protocol::CANSHUNT_PRODUCT,
                                    2,
                                    serial,
                                );
                                let result = async {
                                    let node_id = inquire_direct_node_id(
                                        &mut control_sender,
                                        &mut discovery,
                                        identity,
                                    )
                                    .await?;
                                    tokio::time::sleep(std::time::Duration::from_millis(1100))
                                        .await;
                                    let nmt_state = manager
                                        .node_list()
                                        .await
                                        .into_iter()
                                        .find(|node| node.node_id == node_id)
                                        .and_then(|node| node.nmt_state)
                                        .map(|state| state.to_string());
                                    Ok(Device {
                                        node_id,
                                        vendor_id: Some(identity.vendor_id),
                                        product_code: Some(identity.product_code),
                                        revision: Some(identity.revision),
                                        serial: Some(identity.serial),
                                        software_version: None,
                                        is_canshunt: true,
                                        nmt_state,
                                    })
                                }
                                .await;
                                let _ = reply.send(result);
                            }
                            Command::SetPhysicalCan(enabled, reply) => {
                                let _ = reply
                                    .send(control_sender.set_physical_can_enabled(enabled).await);
                            }
                        }
                    }
                });
            })
            .map_err(|error| format!("Could not start zencan client: {error}"))?;
        Ok(Self { commands })
    }

    async fn request<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<T, String>>) -> Command,
    ) -> Result<T, String> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(make(tx))
            .map_err(|_| "CAN client stopped".to_string())?;
        rx.await.map_err(|_| "CAN client stopped".to_string())?
    }

    pub async fn scan(&self) -> Result<Vec<Device>, String> {
        self.request(Command::Scan).await
    }
    pub async fn read_config(&self, node: u8) -> Result<DeviceConfig, String> {
        self.request(|tx| Command::ReadConfig(node, tx)).await
    }
    pub async fn write_pdos(&self, node: u8, pdos: Vec<PdoConfig>) -> Result<(), String> {
        self.request(|tx| Command::WritePdos(node, pdos, tx)).await
    }
    pub async fn assign(&self, identity: DeviceIdentity, node: u8) -> Result<(), String> {
        self.request(|tx| Command::Assign(identity, node, tx)).await
    }
    pub async fn set_baud(&self, node: u8, value: u8) -> Result<(), String> {
        self.request(|tx| Command::SetBaud(node, value, tx)).await
    }
    pub async fn identify(&self, node: u8) -> Result<(), String> {
        self.request(|tx| Command::Identify(node, tx)).await
    }
    pub async fn poll(&self) -> Result<Vec<RawFrame>, String> {
        self.request(Command::Poll).await
    }
    pub async fn set_nmt(&self, node: u8, state: String) -> Result<String, String> {
        self.request(|tx| Command::SetNmt(node, state, tx)).await
    }
    pub async fn direct_device(&self, serial: u32) -> Result<Device, String> {
        self.request(|tx| Command::DirectDevice(serial, tx)).await
    }
    pub async fn set_physical_can(&self, enabled: bool) -> Result<bool, String> {
        self.request(|tx| Command::SetPhysicalCan(enabled, tx))
            .await
    }
}

async fn inquire_direct_node_id(
    sender: &mut TransportSender,
    receiver: &mut zencan_transport::TransportReceiver,
    identity: LssIdentity,
) -> Result<u8, String> {
    receiver.flush();
    for request in [
        LssRequest::SwitchModeGlobal {
            mode: LssState::Waiting as u8,
        },
        LssRequest::SwitchStateVendor {
            vendor_id: identity.vendor_id,
        },
        LssRequest::SwitchStateProduct {
            product_code: identity.product_code,
        },
        LssRequest::SwitchStateRevision {
            revision: identity.revision,
        },
        LssRequest::SwitchStateSerial {
            serial: identity.serial,
        },
    ] {
        sender
            .send(request.into())
            .await
            .map_err(|error| error.message())?;
    }
    wait_for_lss(receiver, |response| {
        matches!(response, LssResponse::SwitchStateResponse)
    })
    .await?;
    sender
        .send(LssRequest::InquireNodeId.into())
        .await
        .map_err(|error| error.message())?;
    let node_id = wait_for_lss(receiver, |response| {
        matches!(response, LssResponse::InquireNodeIdAck { .. })
    })
    .await?;
    sender
        .send(
            LssRequest::SwitchModeGlobal {
                mode: LssState::Waiting as u8,
            }
            .into(),
        )
        .await
        .map_err(|error| error.message())?;
    match node_id {
        LssResponse::InquireNodeIdAck { node_id } => Ok(node_id),
        _ => unreachable!(),
    }
}

async fn wait_for_lss(
    receiver: &mut zencan_transport::TransportReceiver,
    accept: impl Fn(&LssResponse) -> bool,
) -> Result<LssResponse, String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(250);
    loop {
        let message = tokio::time::timeout_at(deadline, receiver.recv())
            .await
            .map_err(|_| "Timed out waiting for the directly connected CANShunt".to_string())?
            .map_err(|_| "CAN transport stopped".to_string())?;
        if let Ok(response) = LssResponse::try_from(message)
            && accept(&response)
        {
            return Ok(response);
        }
    }
}
