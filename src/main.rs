use std::sync::{Arc, Mutex};

use wayland_client::protocol::{wl_output::WlOutput, wl_seat::WlSeat};
use wayland_client::{Display, GlobalManager, Main};

// Generated bindings for river-status protocol
mod river_status {
    pub use wayland_client::{
        sys,
        AnonymousObject,
        Interface,
        Main,
        MessageGroup,
        Proxy,
        ProxyMap,
        protocol::{wl_output, wl_seat},
    };
    pub use wayland_commons::map::{Object, ObjectMetadata};
    pub use wayland_commons::smallvec;
    pub use wayland_commons::wire::{Argument, ArgumentType, Message, MessageDesc};
    include!(concat!(env!("OUT_DIR"), "/river-status-unstable-v1.rs"));
}

use river_status::zriver_output_status_v1::ZriverOutputStatusV1;
use river_status::zriver_seat_status_v1::ZriverSeatStatusV1;
use river_status::zriver_status_manager_v1::ZriverStatusManagerV1;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let display = Display::connect_to_env()?;
    let mut event_queue = display.create_event_queue();
    let attached_display = display.attach(event_queue.token());

    let _registry = attached_display.get_registry();
    let globals = GlobalManager::new(&attached_display);

    // Gather globals
    event_queue.sync_roundtrip(&mut (), |_, _, _| {})?;

    // Instantiate river status manager if available
    let manager: Option<Main<ZriverStatusManagerV1>> = instantiate_if_present(
        &globals,
        "zriver_status_manager_v1",
        4,
    );

    if manager.is_none() {
        eprintln!("zriver_status_manager_v1 not available");
        return Ok(());
    }
    let manager = manager.unwrap();

    // Instantiate wl_output and wl_seat objects we will monitor
    let outputs: Vec<Main<WlOutput>> = instantiate_all::<WlOutput>(&globals, 3);
    let seats: Vec<Main<WlSeat>> = instantiate_all::<WlSeat>(&globals, 5);

    // Keep status objects alive
    let output_statuses: Arc<Mutex<Vec<Main<ZriverOutputStatusV1>>>> =
        Arc::new(Mutex::new(Vec::new()));
    let seat_statuses: Arc<Mutex<Vec<Main<ZriverSeatStatusV1>>>> =
        Arc::new(Mutex::new(Vec::new()));

    // Create output status listeners
    for out in &outputs {
        let status = manager.get_river_output_status(out);
        setup_output_status_handlers(&status);
        output_statuses.lock().unwrap().push(status);
    }

    // Create seat status listeners
    for seat in &seats {
        let status = manager.get_river_seat_status(seat);
        setup_seat_status_handlers(&status);
        seat_statuses.lock().unwrap().push(status);
    }

    // Roundtrip to receive the initial state
    event_queue.sync_roundtrip(&mut (), |_, _, _| {})?;

    // Dispatch events forever
    loop {
        event_queue.dispatch(&mut (), |_, _, _| {})?;
    }
}

fn instantiate_if_present<T>(globals: &GlobalManager, name: &str, version: u32) -> Option<Main<T>>
where
    T: wayland_client::Interface
        + std::convert::AsRef<wayland_client::Proxy<T>>
        + std::convert::From<wayland_client::Proxy<T>>,
{
    let offered = globals
        .list()
        .iter()
        .find(|(_, iface, _)| iface.as_str() == name)
        .map(|(_, _, ver)| *ver);
    let Some(offered) = offered else { return None; };
    let ver = version.min(offered);
    Some(globals.instantiate_exact::<T>(ver).expect("instantiate exact"))
}

fn instantiate_all<T>(globals: &GlobalManager, version: u32) -> Vec<Main<T>>
where
    T: wayland_client::Interface
        + std::convert::AsRef<wayland_client::Proxy<T>>
        + std::convert::From<wayland_client::Proxy<T>>,
{
    let name = T::NAME;
    let mut out = Vec::new();
    for (_, iface, offered_ver) in globals.list().iter().filter(|(_, iface, _)| iface == name) {
        let ver = version.min(*offered_ver);
        let inst = globals
            .instantiate_exact::<T>(ver)
            .unwrap_or_else(|_| panic!("instantiate {} v{}", name, ver));
        out.push(inst);
    }
    out
}

fn setup_output_status_handlers(status: &Main<ZriverOutputStatusV1>) {
    status.quick_assign(|_status, event, _| {
        use river_status::zriver_output_status_v1::Event;
        match event {
            Event::FocusedTags { tags } => {
                println!("output: focused_tags=0x{tags:08x}");
            }
            Event::ViewTags { tags } => {
                // tags is an array of u32 bitfields packed into bytes.
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
    });
}

fn setup_seat_status_handlers(status: &Main<ZriverSeatStatusV1>) {
    status.quick_assign(|_status, event, _| {
        use river_status::zriver_seat_status_v1::Event;
        match event {
            Event::FocusedOutput { output } => {
                // Log object id to distinguish outputs without xdg-output names.
                println!("seat: focused_output id={}", output.as_ref().id());
            }
            Event::UnfocusedOutput { output } => {
                println!("seat: unfocused_output id={}", output.as_ref().id());
            }
            Event::FocusedView { title } => {
                println!("seat: focused_view title=\"{title}\"");
            }
            Event::Mode { name } => {
                println!("seat: mode=\"{name}\"");
            }
            _ => {}
        }
    });
}

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
