use std::collections::HashMap;

use crate::{
    message::presence::{PresenceDiff, PresenceEvent, PresenceState},
    realtime_channel::PresenceCallback,
};

pub(crate) type PresenceCallbackMap = HashMap<PresenceEvent, Vec<PresenceCallback>>;

#[derive(Default)]
pub(crate) struct RealtimePresence {
    pub state: PresenceState,
    callbacks: PresenceCallbackMap,
}

impl RealtimePresence {
    pub(crate) fn from_channel_builder(
        callbacks: HashMap<PresenceEvent, Vec<PresenceCallback>>,
    ) -> Self {
        Self {
            state: PresenceState::default(),
            callbacks,
        }
    }

    pub(crate) fn sync(&mut self, new_state: PresenceState) {
        let joins: PresenceState = new_state
            .0
            .clone()
            .into_iter()
            .map(|(new_id, mut new_phx_map)| {
                new_phx_map.retain(|new_phx_ref, _new_state_data| {
                    let mut retain = true;
                    let _ = self.state.0.clone().into_values().map(|self_phx_map| {
                        if self_phx_map.contains_key(new_phx_ref) {
                            retain = false;
                        }
                    });
                    retain
                });

                (new_id, new_phx_map)
            })
            .collect();

        let leaves: PresenceState = self
            .state
            .0
            .clone()
            .into_iter()
            .map(|(current_id, mut current_phx_map)| {
                current_phx_map.retain(|current_phx_ref, _current_state_data| {
                    let mut retain = false;
                    let _ = new_state.0.clone().into_values().map(|new_phx_map| {
                        if !new_phx_map.contains_key(current_phx_ref) {
                            retain = true;
                        }
                    });
                    retain
                });

                (current_id, current_phx_map)
            })
            .collect();

        let prev_state = self.state.clone();

        self.sync_diff(PresenceDiff { joins, leaves });

        for (id, _data) in self.state.0.clone() {
            for cb in self
                .callbacks
                .get_mut(&PresenceEvent::Sync)
                .unwrap_or(&mut vec![])
            {
                cb.0(id.clone(), prev_state.clone(), self.state.clone());
            }
        }
    }

    pub(crate) fn sync_diff(&mut self, diff: PresenceDiff) -> &PresenceState {
        // mutate own state with diff
        // return new state
        // trigger diff callbacks

        for (id, _data) in diff.joins.0.clone() {
            for cb in self
                .callbacks
                .get_mut(&PresenceEvent::Join)
                .unwrap_or(&mut vec![])
            {
                cb.0(id.clone(), self.state.clone(), diff.clone().joins);
            }
        }

        for (id, _data) in diff.leaves.0.clone() {
            for cb in self
                .callbacks
                .get_mut(&PresenceEvent::Leave)
                .unwrap_or(&mut vec![])
            {
                cb.0(id.clone(), self.state.clone(), diff.clone().leaves);
            }

            self.state.0.remove(&id);
        }

        self.state.0.extend(diff.joins.0);

        &self.state
    }
}
