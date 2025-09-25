use async_graphql::futures_util::future::ready;
use async_graphql::futures_util::{Stream, StreamExt};
use async_graphql::{Context, EmptyMutation, Enum, ID, Object, Schema, Subscription, Union};
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast::Sender;
use tokio_stream::wrappers::BroadcastStream;

use crate::river;

#[derive(Enum, Copy, Clone, Eq, PartialEq, Hash)]
pub enum RiverEventType {
    OutputFocusedTags,
    OutputViewTags,
    OutputUrgentTags,
    OutputLayoutName,
    OutputLayoutNameClear,
    SeatFocusedOutput,
    SeatUnfocusedOutput,
    SeatFocusedView,
    SeatMode,
}

impl From<&river::Event> for RiverEventType {
    fn from(e: &river::Event) -> Self {
        use river::Event::*;
        match e {
            OutputFocusedTags { .. } => RiverEventType::OutputFocusedTags,
            OutputViewTags { .. } => RiverEventType::OutputViewTags,
            OutputUrgentTags { .. } => RiverEventType::OutputUrgentTags,
            OutputLayoutName { .. } => RiverEventType::OutputLayoutName,
            OutputLayoutNameClear => RiverEventType::OutputLayoutNameClear,
            SeatFocusedOutput { .. } => RiverEventType::SeatFocusedOutput,
            SeatUnfocusedOutput { .. } => RiverEventType::SeatUnfocusedOutput,
            SeatFocusedView { .. } => RiverEventType::SeatFocusedView,
            SeatMode { .. } => RiverEventType::SeatMode,
        }
    }
}

#[derive(Default, Clone)]
pub struct RiverSnapshot {
    pub output_focused_tags: Option<i32>,
    pub output_view_tags: Option<Vec<i32>>,
    pub output_urgent_tags: Option<i32>,
    pub output_layout_name: Option<String>,
    pub seat_focused_output: Option<NamedOutputId>,
    pub seat_unfocused_output: Option<NamedOutputId>,
    pub seat_focused_view: Option<String>,
    pub seat_mode: Option<String>,
}

#[derive(Clone)]
pub struct NamedOutputId {
    pub output_id: ID,
    pub name: Option<String>,
}

impl RiverSnapshot {
    pub fn apply_event(&mut self, event: &river::Event) {
        use river::Event::*;
        match event {
            OutputFocusedTags { tags } => {
                self.output_focused_tags = Some(*tags as i32);
            }
            OutputViewTags { tags } => {
                let converted = tags.iter().map(|v| *v as i32).collect::<Vec<i32>>();
                self.output_view_tags = Some(converted);
            }
            OutputUrgentTags { tags } => {
                self.output_urgent_tags = Some(*tags as i32);
            }
            OutputLayoutName { name } => {
                self.output_layout_name = Some(name.clone());
            }
            OutputLayoutNameClear => {
                self.output_layout_name = None;
            }
            SeatFocusedOutput { id, name } => {
                self.seat_focused_output = Some(NamedOutputId {
                    output_id: id_to_graphql(id),
                    name: name.clone(),
                });
            }
            SeatUnfocusedOutput { id, name } => {
                self.seat_unfocused_output = Some(NamedOutputId {
                    output_id: id_to_graphql(id),
                    name: name.clone(),
                });
            }
            SeatFocusedView { title } => {
                self.seat_focused_view = Some(title.clone());
            }
            SeatMode { name } => {
                self.seat_mode = Some(name.clone());
            }
        }
    }
}

pub type RiverStateHandle = Arc<RwLock<RiverSnapshot>>;

pub fn new_river_state() -> RiverStateHandle {
    Arc::new(RwLock::new(RiverSnapshot::default()))
}

pub fn update_river_state(handle: &RiverStateHandle, event: &river::Event) {
    if let Ok(mut state) = handle.write() {
        state.apply_event(event);
    }
}

#[derive(Clone)]
pub struct OutputIdPayload {
    pub output_id: String,
}

#[derive(Clone)]
pub struct SeatIdPayload {
    pub seat_id: String,
}

#[derive(Union, Clone)]
pub enum RiverEvent {
    OutputFocusedTags(GOutputFocusedTags),
    OutputViewTags(GOutputViewTags),
    OutputUrgentTags(GOutputUrgentTags),
    OutputLayoutName(GOutputLayoutName),
    SeatFocusedOutput(GSeatFocusedOutput),
    SeatUnfocusedOutput(GSeatUnfocusedOutput),
    SeatFocusedView(GSeatFocusedView),
    SeatMode(GSeatMode),
}

#[derive(Clone)]
pub struct GOutputFocusedTags {
    pub tags: i32,
}
#[Object(name = "OutputFocusedTags")]
impl GOutputFocusedTags {
    async fn tags(&self) -> i32 {
        self.tags
    }
}

#[derive(Clone)]
pub struct GOutputViewTags {
    pub tags: Vec<i32>,
}
#[Object(name = "OutputViewTags")]
impl GOutputViewTags {
    async fn tags(&self) -> &Vec<i32> {
        &self.tags
    }
}

#[derive(Clone)]
pub struct GOutputUrgentTags {
    pub tags: i32,
}
#[Object(name = "OutputUrgentTags")]
impl GOutputUrgentTags {
    async fn tags(&self) -> i32 {
        self.tags
    }
}

#[derive(Clone)]
pub struct GOutputLayoutName {
    pub name: String,
}
#[Object(name = "OutputLayoutName")]
impl GOutputLayoutName {
    async fn name(&self) -> &str {
        &self.name
    }
}

// no-op clear event omitted in minimal schema

#[derive(Clone)]
pub struct GSeatFocusedOutput {
    pub output_id: ID,
    pub name: Option<String>,
}
#[Object(name = "SeatFocusedOutput")]
impl GSeatFocusedOutput {
    async fn output_id(&self) -> &ID {
        &self.output_id
    }

    async fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

#[derive(Clone)]
pub struct GSeatUnfocusedOutput {
    pub output_id: ID,
    pub name: Option<String>,
}
#[Object(name = "SeatUnfocusedOutput")]
impl GSeatUnfocusedOutput {
    async fn output_id(&self) -> &ID {
        &self.output_id
    }

    async fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

#[derive(Clone)]
pub struct GSeatFocusedView {
    pub title: String,
}
#[Object(name = "SeatFocusedView")]
impl GSeatFocusedView {
    async fn title(&self) -> &str {
        &self.title
    }
}

#[derive(Clone)]
pub struct GSeatMode {
    pub name: String,
}
#[Object(name = "SeatMode")]
impl GSeatMode {
    async fn name(&self) -> &str {
        &self.name
    }
}

fn id_to_graphql(id: &wayland_backend::client::ObjectId) -> ID {
    ID(id.to_string())
}

impl From<river::Event> for RiverEvent {
    fn from(value: river::Event) -> Self {
        use river::Event::*;
        match value {
            OutputFocusedTags { tags } => {
                RiverEvent::OutputFocusedTags(GOutputFocusedTags { tags: tags as i32 })
            }
            OutputViewTags { tags } => RiverEvent::OutputViewTags(GOutputViewTags {
                tags: tags.into_iter().map(|v| v as i32).collect::<Vec<i32>>(),
            }),
            OutputUrgentTags { tags } => {
                RiverEvent::OutputUrgentTags(GOutputUrgentTags { tags: tags as i32 })
            }
            OutputLayoutName { name } => RiverEvent::OutputLayoutName(GOutputLayoutName { name }),
            OutputLayoutNameClear => RiverEvent::OutputLayoutName(GOutputLayoutName {
                name: String::new(),
            }),
            SeatFocusedOutput {
                id: output_id,
                name,
            } => RiverEvent::SeatFocusedOutput(GSeatFocusedOutput {
                output_id: id_to_graphql(&output_id),
                name,
            }),
            SeatUnfocusedOutput {
                id: output_id,
                name,
            } => RiverEvent::SeatUnfocusedOutput(GSeatUnfocusedOutput {
                output_id: id_to_graphql(&output_id),
                name,
            }),
            SeatFocusedView { title } => RiverEvent::SeatFocusedView(GSeatFocusedView { title }),
            SeatMode { name } => RiverEvent::SeatMode(GSeatMode { name }),
        }
    }
}

pub struct QueryRoot;
#[Object]
impl QueryRoot {
    async fn hello(&self) -> &str {
        "ok"
    }

    async fn output_focused_tags(&self, ctx: &Context<'_>) -> Option<GOutputFocusedTags> {
        let handle = ctx.data_unchecked::<RiverStateHandle>();
        let Ok(snapshot) = handle.read() else {
            return None;
        };
        snapshot
            .output_focused_tags
            .map(|tags| GOutputFocusedTags { tags })
    }

    async fn output_view_tags(&self, ctx: &Context<'_>) -> Option<GOutputViewTags> {
        let handle = ctx.data_unchecked::<RiverStateHandle>();
        let Ok(snapshot) = handle.read() else {
            return None;
        };
        snapshot
            .output_view_tags
            .clone()
            .map(|tags| GOutputViewTags { tags })
    }

    async fn output_urgent_tags(&self, ctx: &Context<'_>) -> Option<GOutputUrgentTags> {
        let handle = ctx.data_unchecked::<RiverStateHandle>();
        let Ok(snapshot) = handle.read() else {
            return None;
        };
        snapshot
            .output_urgent_tags
            .map(|tags| GOutputUrgentTags { tags })
    }

    async fn output_layout_name(&self, ctx: &Context<'_>) -> Option<GOutputLayoutName> {
        let handle = ctx.data_unchecked::<RiverStateHandle>();
        let Ok(snapshot) = handle.read() else {
            return None;
        };
        snapshot
            .output_layout_name
            .clone()
            .map(|name| GOutputLayoutName { name })
    }

    async fn seat_focused_output(&self, ctx: &Context<'_>) -> Option<GSeatFocusedOutput> {
        let handle = ctx.data_unchecked::<RiverStateHandle>();
        let Ok(snapshot) = handle.read() else {
            return None;
        };
        snapshot
            .seat_focused_output
            .clone()
            .map(|named| GSeatFocusedOutput {
                output_id: named.output_id,
                name: named.name,
            })
    }

    async fn seat_unfocused_output(&self, ctx: &Context<'_>) -> Option<GSeatUnfocusedOutput> {
        let handle = ctx.data_unchecked::<RiverStateHandle>();
        let Ok(snapshot) = handle.read() else {
            return None;
        };
        snapshot
            .seat_unfocused_output
            .clone()
            .map(|named| GSeatUnfocusedOutput {
                output_id: named.output_id,
                name: named.name,
            })
    }

    async fn seat_focused_view(&self, ctx: &Context<'_>) -> Option<GSeatFocusedView> {
        let handle = ctx.data_unchecked::<RiverStateHandle>();
        let Ok(snapshot) = handle.read() else {
            return None;
        };
        snapshot
            .seat_focused_view
            .clone()
            .map(|title| GSeatFocusedView { title })
    }

    async fn seat_mode(&self, ctx: &Context<'_>) -> Option<GSeatMode> {
        let handle = ctx.data_unchecked::<RiverStateHandle>();
        let Ok(snapshot) = handle.read() else {
            return None;
        };
        snapshot.seat_mode.clone().map(|name| GSeatMode { name })
    }
}

pub struct SubscriptionRoot;
#[Subscription]
impl SubscriptionRoot {
    async fn river_events(
        &self,
        ctx: &Context<'_>,
        types: Option<Vec<RiverEventType>>,
    ) -> impl Stream<Item = RiverEvent> {
        let sender = ctx.data_unchecked::<Sender<river::Event>>().clone();
        let rx = sender.subscribe();
        let tset = types.map(|v| v.into_iter().collect::<std::collections::HashSet<_>>());
        BroadcastStream::new(rx).filter_map(move |item| {
            let e = match item {
                Ok(ev) => ev,
                Err(_) => return ready(None),
            };
            let pass = tset
                .as_ref()
                .map_or(true, |ts| ts.contains(&RiverEventType::from(&e)));
            if pass {
                ready(Some(RiverEvent::from(e)))
            } else {
                ready(None)
            }
        })
    }
}

pub type AppSchema = Schema<QueryRoot, EmptyMutation, SubscriptionRoot>;
