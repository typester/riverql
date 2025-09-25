use std::collections::HashMap;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use wayland_client::protocol::{
    wl_output::{self, WlOutput},
    wl_registry,
    wl_registry::WlRegistry,
    wl_seat::WlSeat,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};

pub mod river_status {
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocol/river-status-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("protocol/river-status-unstable-v1.xml");
}

use river_status::zriver_output_status_v1::ZriverOutputStatusV1;
use river_status::zriver_seat_status_v1::ZriverSeatStatusV1;
use river_status::zriver_status_manager_v1::ZriverStatusManagerV1;
use wayland_backend::client::ObjectId;

#[derive(Debug, Clone)]
pub enum Event {
    OutputFocusedTags {
        id: ObjectId,
        name: Option<String>,
        tags: u32,
    },
    OutputViewTags {
        id: ObjectId,
        name: Option<String>,
        tags: Vec<u32>,
    },
    OutputUrgentTags {
        id: ObjectId,
        name: Option<String>,
        tags: u32,
    },
    OutputLayoutName {
        id: ObjectId,
        name: Option<String>,
        layout: String,
    },
    OutputLayoutNameClear {
        id: ObjectId,
        name: Option<String>,
    },

    SeatFocusedOutput {
        id: ObjectId,
        name: Option<String>,
    },
    SeatUnfocusedOutput {
        id: ObjectId,
        name: Option<String>,
    },
    SeatFocusedView {
        title: String,
    },
    SeatMode {
        name: String,
    },
}

struct State {
    outputs: Vec<WlOutput>,
    seats: Vec<WlSeat>,
    manager: Option<ZriverStatusManagerV1>,
    output_statuses: Vec<ZriverOutputStatusV1>,
    seat_statuses: Vec<ZriverSeatStatusV1>,
    tx: UnboundedSender<Event>,
    output_info: HashMap<u32, OutputInfo>,
    output_status_owner: HashMap<u32, ObjectId>,
}

impl State {
    fn new(tx: UnboundedSender<Event>) -> Self {
        Self {
            outputs: Vec::new(),
            seats: Vec::new(),
            manager: None,
            output_statuses: Vec::new(),
            seat_statuses: Vec::new(),
            tx,
            output_info: HashMap::new(),
            output_status_owner: HashMap::new(),
        }
    }

    fn maybe_create_status_for_output(&mut self, qh: &QueueHandle<Self>, out: &WlOutput) {
        if let Some(ref mgr) = self.manager {
            let status = mgr.get_river_output_status(out, qh, ());
            let status_id = status.id().protocol_id();
            let output_id = out.id();
            self.output_status_owner.insert(status_id, output_id);
            self.output_statuses.push(status);
        }
        let id = out.id().protocol_id();
        self.output_info.entry(id).or_default();
    }

    fn maybe_create_status_for_seat(&mut self, qh: &QueueHandle<Self>, seat: &WlSeat) {
        if let Some(ref mgr) = self.manager {
            let st = mgr.get_river_seat_status(seat, qh, ());
            self.seat_statuses.push(st);
        }
    }

    fn create_status_for_all(&mut self, qh: &QueueHandle<Self>) {
        if self.manager.is_some() {
            let outs = self.outputs.clone();
            for o in &outs {
                self.maybe_create_status_for_output(qh, o);
            }
            let seats = self.seats.clone();
            for s in &seats {
                self.maybe_create_status_for_seat(qh, s);
            }
        }
    }

    fn update_output_info(&mut self, id: &ObjectId, update: impl FnOnce(&mut OutputInfo)) {
        let entry = self
            .output_info
            .entry(id.protocol_id())
            .or_insert_with(OutputInfo::default);
        update(entry);
    }

    fn output_label(&self, id: &ObjectId) -> Option<String> {
        self.output_info
            .get(&id.protocol_id())
            .and_then(|info| info.label())
    }
}

#[derive(Debug, Default, Clone)]
struct OutputInfo {
    name: Option<String>,
    description: Option<String>,
    make: Option<String>,
    model: Option<String>,
}

impl OutputInfo {
    fn label(&self) -> Option<String> {
        if let Some(name) = &self.name {
            if !name.is_empty() {
                return Some(name.clone());
            }
        }
        if let Some(desc) = &self.description {
            if !desc.is_empty() {
                return Some(desc.clone());
            }
        }
        match (&self.make, &self.model) {
            (Some(make), Some(model)) if !make.is_empty() || !model.is_empty() => {
                Some(format!("{make} {model}").trim().to_string())
            }
            (Some(make), None) if !make.is_empty() => Some(make.clone()),
            _ => None,
        }
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_output" => {
                    let output = registry.bind::<WlOutput, _, _>(name, version.min(4), qh, ());
                    state.outputs.push(output);
                    let last = state.outputs.last().unwrap().clone();
                    state.maybe_create_status_for_output(qh, &last);
                }
                "wl_seat" => {
                    let seat = registry.bind::<WlSeat, _, _>(name, version.min(5), qh, ());
                    state.seats.push(seat);
                    let last = state.seats.last().unwrap().clone();
                    state.maybe_create_status_for_seat(qh, &last);
                }
                "zriver_status_manager_v1" => {
                    let mgr =
                        registry.bind::<ZriverStatusManagerV1, _, _>(name, version.min(4), qh, ());
                    state.manager = Some(mgr);
                    state.create_status_for_all(qh);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let id = proxy.id();
        match event {
            wl_output::Event::Name { name } => {
                state.update_output_info(&id, |info| info.name = Some(name));
            }
            wl_output::Event::Description { description } => {
                state.update_output_info(&id, |info| info.description = Some(description));
            }
            wl_output::Event::Geometry { make, model, .. } => {
                state.update_output_info(&id, |info| {
                    info.make = Some(make);
                    info.model = Some(model);
                });
            }
            _ => {}
        }
    }
}

impl Dispatch<ZriverOutputStatusV1, ()> for State {
    fn event(
        state: &mut Self,
        status: &ZriverOutputStatusV1,
        event: river_status::zriver_output_status_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use river_status::zriver_output_status_v1::Event as E;
        let Some(output_id) = state
            .output_status_owner
            .get(&status.id().protocol_id())
            .cloned()
        else {
            return;
        };
        let label = state.output_label(&output_id);
        match event {
            E::FocusedTags { tags } => {
                let _ = state.tx.send(Event::OutputFocusedTags {
                    id: output_id,
                    name: label,
                    tags,
                });
            }
            E::ViewTags { tags } => {
                let parsed = parse_u32_array(&tags);
                let _ = state.tx.send(Event::OutputViewTags {
                    id: output_id,
                    name: label,
                    tags: parsed,
                });
            }
            E::UrgentTags { tags } => {
                let _ = state.tx.send(Event::OutputUrgentTags {
                    id: output_id,
                    name: label,
                    tags,
                });
            }
            E::LayoutName { name } => {
                let _ = state.tx.send(Event::OutputLayoutName {
                    id: output_id,
                    name: label,
                    layout: name,
                });
            }
            E::LayoutNameClear => {
                let _ = state.tx.send(Event::OutputLayoutNameClear {
                    id: output_id,
                    name: label,
                });
            }
        }
    }
}

impl Dispatch<ZriverSeatStatusV1, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &ZriverSeatStatusV1,
        event: river_status::zriver_seat_status_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use river_status::zriver_seat_status_v1::Event as E;
        match event {
            E::FocusedOutput { output } => {
                let id = output.id();
                let label = state.output_label(&id);
                let _ = state.tx.send(Event::SeatFocusedOutput { id, name: label });
            }
            E::UnfocusedOutput { output } => {
                let id = output.id();
                let label = state.output_label(&id);
                let _ = state
                    .tx
                    .send(Event::SeatUnfocusedOutput { id, name: label });
            }
            E::FocusedView { title } => {
                let _ = state.tx.send(Event::SeatFocusedView { title });
            }
            E::Mode { name } => {
                let _ = state.tx.send(Event::SeatMode { name });
            }
        }
    }
}

delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ignore ZriverStatusManagerV1);

fn parse_u32_array(bytes: &[u8]) -> Vec<u32> {
    let mut v = Vec::new();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let chunk = [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]];
        v.push(u32::from_ne_bytes(chunk));
        i += 4;
    }
    v
}

pub struct RiverStatus;

impl RiverStatus {
    pub fn subscribe() -> Result<UnboundedReceiver<Event>, Box<dyn std::error::Error>> {
        let conn = Connection::connect_to_env()?;
        let (tx, rx) = mpsc::unbounded_channel();

        let mut state = State::new(tx);
        let mut event_queue: EventQueue<State> = conn.new_event_queue();
        let qh = event_queue.handle();

        let display = conn.display();
        let _registry = display.get_registry(&qh, ());

        event_queue.roundtrip(&mut state)?;

        std::thread::spawn(move || {
            let mut blocking_queue = event_queue;
            loop {
                if let Err(_e) = blocking_queue.blocking_dispatch(&mut state) {
                    break;
                }
            }
        });

        Ok(rx)
    }
}
