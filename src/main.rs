use wayland_client::protocol::{
    wl_output::WlOutput, wl_registry, wl_registry::WlRegistry, wl_seat::WlSeat,
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

struct State {
    outputs: Vec<WlOutput>,
    seats: Vec<WlSeat>,
    manager: Option<ZriverStatusManagerV1>,
    output_statuses: Vec<ZriverOutputStatusV1>,
    seat_statuses: Vec<ZriverSeatStatusV1>,
}

impl State {
    fn new() -> Self {
        Self {
            outputs: Vec::new(),
            seats: Vec::new(),
            manager: None,
            output_statuses: Vec::new(),
            seat_statuses: Vec::new(),
        }
    }

    fn maybe_create_status_for_output(&mut self, qh: &QueueHandle<Self>, out: &WlOutput) {
        if let Some(ref mgr) = self.manager {
            let st = mgr.get_river_output_status(out, qh, ());
            self.output_statuses.push(st);
        }
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
                    let output = registry.bind::<WlOutput, _, _>(name, version.min(3), qh, ());
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
            wl_registry::Event::GlobalRemove { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<ZriverOutputStatusV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &ZriverOutputStatusV1,
        event: river_status::zriver_output_status_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use river_status::zriver_output_status_v1::Event;
        match event {
            Event::FocusedTags { tags } => {
                println!("output: focused_tags=0x{tags:08x}");
            }
            Event::ViewTags { tags } => {
                let parsed = parse_u32_array(&tags);
                print!("output: view_tags=[");
                for (i, v) in parsed.iter().enumerate() {
                    if i > 0 {
                        print!(", ");
                    }
                    print!("0x{v:08x}");
                }
                println!("]");
            }
            Event::UrgentTags { tags } => {
                println!("output: urgent_tags=0x{tags:08x}");
            }
            Event::LayoutName { name } => {
                println!("output: layout_name=\"{name}\"");
            }
            Event::LayoutNameClear => {
                println!("output: layout_name_clear");
            }
            _ => {}
        }
    }
}

impl Dispatch<ZriverSeatStatusV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &ZriverSeatStatusV1,
        event: river_status::zriver_seat_status_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use river_status::zriver_seat_status_v1::Event;
        match event {
            Event::FocusedOutput { output } => {
                println!("seat: focused_output id={}", output.id());
            }
            Event::UnfocusedOutput { output } => {
                println!("seat: unfocused_output id={}", output.id());
            }
            Event::FocusedView { title } => {
                println!("seat: focused_view title=\"{title}\"");
            }
            Event::Mode { name } => {
                println!("seat: mode=\"{name}\"");
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore WlOutput);
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let mut state = State::new();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();

    let display = conn.display();
    let _registry = display.get_registry(&qh, ());

    event_queue.roundtrip(&mut state)?;
    loop {
        event_queue.blocking_dispatch(&mut state)?;
    }
}
