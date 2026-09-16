use serde::Serialize;
use serde_json::Value;
use std::{collections::VecDeque, sync::{Arc, Mutex}};
use tokio::sync::broadcast;
use uuid::Uuid;

pub trait EventSink: Send + Sync {
    fn emit_value(&self, channel: &str, payload: Value);
}

pub fn emit_event<S: EventSink, T: Serialize>(sink: &S, channel: &str, payload: &T) {
    match serde_json::to_value(payload) {
        Ok(payload) => sink.emit_value(channel, payload),
        Err(error) => tracing::warn!(%error, channel, "failed to serialize event payload"),
    }
}

#[cfg(feature = "desktop")]
impl EventSink for tauri::AppHandle {
    fn emit_value(&self, channel: &str, payload: Value) {
        use tauri::Emitter;
        if let Err(error) = self.emit(channel, payload) {
            tracing::warn!(%error, channel, "failed to emit desktop event");
        }
    }
}

const DEFAULT_REPLAY_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventEnvelope {
    pub generation: String,
    pub sequence: u64,
    pub channel: String,
    pub payload: Value,
}

#[derive(Clone)]
pub struct EventBus {
    state: Arc<Mutex<EventState>>,
    capacity: usize,
    generation: String,
    sender: broadcast::Sender<EventEnvelope>,
}

struct EventState {
    next_sequence: u64,
    history: VecDeque<EventEnvelope>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::with_generation(
            DEFAULT_REPLAY_CAPACITY,
            Uuid::new_v4().simple().to_string(),
        )
    }
}

impl EventBus {
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_generation(capacity, "test".to_string())
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    fn with_generation(capacity: usize, generation: String) -> Self {
        let capacity = capacity.max(1);
        let (sender, _) = broadcast::channel(capacity);
        Self {
            state: Arc::new(Mutex::new(EventState {
                next_sequence: 0,
                history: VecDeque::with_capacity(capacity),
            })),
            capacity,
            generation,
            sender,
        }
    }

    #[cfg(test)]
    fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.sender.subscribe()
    }

    fn publish_value(&self, channel: &str, payload: Value) -> EventEnvelope {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.next_sequence = state.next_sequence.saturating_add(1);
        let envelope = EventEnvelope {
            generation: self.generation.clone(),
            sequence: state.next_sequence,
            channel: channel.to_string(),
            payload,
        };
        if state.history.len() == self.capacity {
            state.history.pop_front();
        }
        state.history.push_back(envelope.clone());
        let _ = self.sender.send(envelope.clone());
        envelope
    }

    pub fn publish<T: Serialize>(
        &self,
        channel: &str,
        payload: &T,
    ) -> Result<EventEnvelope, serde_json::Error> {
        Ok(self.publish_value(channel, serde_json::to_value(payload)?))
    }

    pub fn replay_and_subscribe(
        &self,
        after: u64,
        generation: Option<&str>,
        channels: &[&str],
    ) -> (Vec<EventEnvelope>, broadcast::Receiver<EventEnvelope>, bool) {
        let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let gap = generation.is_some_and(|generation| generation != self.generation)
            || (after > 0
            && state
                .history
                .front()
                .is_some_and(|event| event.sequence > after.saturating_add(1)));
        let replay = state
            .history
            .iter()
            .filter(|event| {
                event.sequence > after
                    && (channels.is_empty()
                        || channels.iter().any(|channel| *channel == event.channel))
            })
            .cloned()
            .collect();
        // The state lock prevents a publisher from advancing history between
        // the replay snapshot and receiver registration.
        (replay, self.sender.subscribe(), gap)
    }

    pub fn replay_after(&self, after: u64, channels: &[&str]) -> Vec<EventEnvelope> {
        let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .history
            .iter()
            .filter(|event| {
                event.sequence > after
                    && (channels.is_empty()
                        || channels.iter().any(|channel| *channel == event.channel))
            })
            .cloned()
            .collect()
    }
}

impl EventSink for EventBus {
    fn emit_value(&self, channel: &str, payload: Value) {
        self.publish_value(channel, payload);
    }
}

impl<T: EventSink + ?Sized> EventSink for Arc<T> {
    fn emit_value(&self, channel: &str, payload: Value) {
        (**self).emit_value(channel, payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn replay_is_sequenced_bounded_and_channel_filtered() {
        let bus = EventBus::with_capacity(2);
        bus.publish("audit:event", &json!({ "step": 1 })).unwrap();
        bus.publish("assistant:event", &json!({ "step": 2 })).unwrap();
        bus.publish("audit:event", &json!({ "step": 3 })).unwrap();

        let replay = bus.replay_after(0, &["audit:event"]);
        assert_eq!(
            replay
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![3]
        );
        assert_eq!(replay[0].payload, json!({ "step": 3 }));
        assert_eq!(
            bus.replay_after(1, &[])
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[tokio::test]
    async fn live_subscribers_receive_the_same_sequenced_envelope() {
        let bus = EventBus::with_capacity(4);
        let mut receiver = bus.subscribe();

        bus.publish("tracks:write-event", &json!({ "current": 1 }))
            .unwrap();

        let event = receiver.recv().await.unwrap();
        assert_eq!(event.sequence, 1);
        assert_eq!(event.channel, "tracks:write-event");
        assert_eq!(event.payload, json!({ "current": 1 }));
    }

    #[test]
    fn event_sink_publishes_transport_payloads() {
        let bus = EventBus::default();
        emit_event(&bus, "assistant:event", &json!({ "type": "step" }));

        assert_eq!(
            bus.replay_after(0, &["assistant:event"])[0].payload,
            json!({ "type": "step" })
        );
    }

    #[test]
    fn reports_when_a_reconnect_is_older_than_the_replay_window() {
        let bus = EventBus::with_capacity(2);
        for step in 1..=4 {
            bus.publish("audit:event", &json!({ "step": step })).unwrap();
        }

        let (_, _, gap) = bus.replay_and_subscribe(1, Some("test"), &["audit:event"]);
        assert!(gap);
    }

    #[test]
    fn reports_a_generation_mismatch_after_a_server_restart() {
        let bus = EventBus::with_generation(2, "new".to_string());

        let (_, _, gap) = bus.replay_and_subscribe(42, Some("old"), &[]);

        assert!(gap);
    }
}
