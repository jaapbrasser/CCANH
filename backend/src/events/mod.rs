//! Typed internal event bus (SPEC §3.2). Domain code publishes; the WebSocket
//! layer subscribes and forwards events to clients allowed to see them.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::cameras::{CameraStatusView, CameraSummary};
use crate::streaming::StreamInfo;
use crate::tasks::Task;

/// Subscription topics a WebSocket client can ask for.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    /// Live status of every camera.
    CameraStatus,
    /// Cameras added, changed or removed.
    Cameras,
    /// Background task progress and completion (own tasks; admins see all).
    Tasks,
    /// Active stream metrics, about once a second (admins only).
    Streams,
}

/// A domain event. Serialized as-is into WebSocket server messages.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    CameraStatus {
        camera_id: Uuid,
        status: CameraStatusView,
    },
    CameraChanged {
        camera: CameraSummary,
    },
    CameraDeleted {
        camera_id: Uuid,
    },
    TaskUpdated {
        task: Task,
    },
    StreamsUpdated {
        streams: Vec<StreamInfo>,
    },
}

impl Event {
    pub fn topic(&self) -> Topic {
        match self {
            Event::CameraStatus { .. } => Topic::CameraStatus,
            Event::CameraChanged { .. } | Event::CameraDeleted { .. } => Topic::Cameras,
            Event::TaskUpdated { .. } => Topic::Tasks,
            Event::StreamsUpdated { .. } => Topic::Streams,
        }
    }
}

/// How many events a slow subscriber may fall behind before it is told to resync.
const CAPACITY: usize = 1024;

#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl Default for EventBus {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(CAPACITY);
        Self { tx }
    }
}

impl EventBus {
    /// Publish to all current subscribers. Having no subscribers is fine.
    pub fn publish(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_receive_published_events() {
        let bus = EventBus::default();
        bus.publish(Event::CameraDeleted {
            camera_id: Uuid::nil(),
        }); // no subscribers yet: dropped, no error
        let mut rx = bus.subscribe();
        let event = Event::CameraDeleted {
            camera_id: Uuid::now_v7(),
        };
        bus.publish(event.clone());
        assert_eq!(rx.recv().await.unwrap(), event);
        assert_eq!(event.topic(), Topic::Cameras);
    }

    #[test]
    fn events_serialize_with_type_tag() {
        let json = serde_json::to_value(Event::CameraDeleted {
            camera_id: Uuid::nil(),
        })
        .unwrap();
        assert_eq!(json["type"], "camera_deleted");
    }
}
