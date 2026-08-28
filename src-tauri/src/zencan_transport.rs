use crate::protocol::{CanBus, CanFrame};
use std::{sync::mpsc, thread, time::Duration};
use tokio::sync::{
    mpsc::{UnboundedReceiver, unbounded_channel},
    oneshot,
};
use zencan_client::common::{
    CanId, CanMessage,
    traits::{AsyncCanReceiver, AsyncCanSender, CanSendError},
};

#[derive(Debug)]
pub struct TransportSender {
    tx: mpsc::Sender<TransportCommand>,
}

enum TransportCommand {
    Frame(CanMessage),
    SetPhysicalCan(bool, oneshot::Sender<Result<bool, String>>),
}

impl Clone for TransportSender {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

#[derive(Debug)]
pub struct TransportSendError(CanMessage);

impl CanSendError for TransportSendError {
    fn into_can_message(self) -> CanMessage {
        self.0
    }
    fn message(&self) -> String {
        "CAN transport is disconnected".into()
    }
}

impl AsyncCanSender for TransportSender {
    type Error = TransportSendError;

    async fn send(&mut self, msg: CanMessage) -> Result<(), Self::Error> {
        self.tx
            .send(TransportCommand::Frame(msg))
            .map_err(|error| match error.0 {
                TransportCommand::Frame(message) => TransportSendError(message),
                TransportCommand::SetPhysicalCan(_, _) => unreachable!(),
            })
    }
}

impl TransportSender {
    pub async fn set_physical_can_enabled(&self, enabled: bool) -> Result<bool, String> {
        let (reply, response) = oneshot::channel();
        self.tx
            .send(TransportCommand::SetPhysicalCan(enabled, reply))
            .map_err(|_| "CAN transport is disconnected".to_string())?;
        response
            .await
            .map_err(|_| "CAN transport is disconnected".to_string())?
    }
}

#[derive(Debug)]
pub struct TransportReceiver {
    rx: UnboundedReceiver<CanMessage>,
}

#[derive(Debug)]
pub struct TransportReceiveError;

impl AsyncCanReceiver for TransportReceiver {
    type Error = TransportReceiveError;

    fn try_recv(&mut self) -> Option<CanMessage> {
        self.rx.try_recv().ok()
    }

    async fn recv(&mut self) -> Result<CanMessage, Self::Error> {
        self.rx.recv().await.ok_or(TransportReceiveError)
    }
}

pub fn start(
    mut bus: Box<dyn CanBus>,
) -> (
    TransportSender,
    TransportReceiver,
    TransportReceiver,
    TransportReceiver,
) {
    let (send_tx, send_rx) = mpsc::channel::<TransportCommand>();
    let (manager_tx, manager_rx) = unbounded_channel();
    let (monitor_tx, monitor_rx) = unbounded_channel();
    let (discovery_tx, discovery_rx) = unbounded_channel();

    thread::Builder::new()
        .name("canshunt-can".into())
        .spawn(move || {
            loop {
                while let Ok(command) = send_rx.try_recv() {
                    match command {
                        TransportCommand::Frame(message) => {
                            let id = message.id();
                            let frame = CanFrame {
                                id: id.raw(),
                                extended: id.is_extended(),
                                data: message.data().to_vec(),
                            };
                            if bus.send(&frame).is_err() {
                                return;
                            }
                        }
                        TransportCommand::SetPhysicalCan(enabled, reply) => {
                            let _ = reply.send(bus.set_physical_can_enabled(enabled));
                        }
                    }
                }
                match bus.receive(Duration::from_millis(5)) {
                    Ok(Some(frame)) => {
                        let id = if frame.extended {
                            CanId::extended(frame.id)
                        } else {
                            CanId::std(frame.id as u16)
                        };
                        let message = CanMessage::new(id, &frame.data);
                        if manager_tx.send(message).is_err() {
                            return;
                        }
                        let _ = monitor_tx.send(message);
                        let _ = discovery_tx.send(message);
                    }
                    Ok(None) => {}
                    Err(_) => return,
                }
                if manager_tx.is_closed() {
                    return;
                }
            }
        })
        .expect("failed to start CAN transport thread");

    (
        TransportSender { tx: send_tx },
        TransportReceiver { rx: manager_rx },
        TransportReceiver { rx: discovery_rx },
        TransportReceiver { rx: monitor_rx },
    )
}
