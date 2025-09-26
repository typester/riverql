use async_graphql::futures_util::future::ready;
use async_graphql::futures_util::{Stream, StreamExt};
use async_graphql::{Context, EmptyMutation, Enum, ID, Object, Schema, Subscription, Union};
use std::collections::HashMap;
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
            OutputLayoutNameClear { .. } => RiverEventType::OutputLayoutNameClear,
            SeatFocusedOutput { .. } => RiverEventType::SeatFocusedOutput,
            SeatUnfocusedOutput { .. } => RiverEventType::SeatUnfocusedOutput,
            SeatFocusedView { .. } => RiverEventType::SeatFocusedView,
            SeatMode { .. } => RiverEventType::SeatMode,
        }
    }
}

#[derive(Default, Clone)]
pub struct RiverSnapshot {
    pub outputs: HashMap<String, OutputState>,
    output_names: HashMap<String, String>,
    pub seat_focused_output: Option<NamedOutputId>,
    pub seat_focused_view: Option<String>,
    pub seat_mode: Option<String>,
}

#[derive(Clone)]
pub struct NamedOutputId {
    pub output_id: ID,
    pub name: Option<String>,
}

#[derive(Clone)]
pub struct OutputState {
    pub output_id: ID,
    pub name: Option<String>,
    pub focused_tags: Option<i32>,
    pub view_tags: Option<Vec<i32>>,
    pub urgent_tags: Option<i32>,
    pub layout_name: Option<String>,
}

#[derive(Clone)]
pub struct GOutputState {
    pub output_id: ID,
    pub name: Option<String>,
    pub focused_tags: Option<i32>,
    pub view_tags: Option<Vec<i32>>,
    pub urgent_tags: Option<i32>,
    pub layout_name: Option<String>,
}

impl From<OutputState> for GOutputState {
    fn from(state: OutputState) -> Self {
        Self::from(&state)
    }
}

impl From<&OutputState> for GOutputState {
    fn from(state: &OutputState) -> Self {
        Self {
            output_id: state.output_id.clone(),
            name: state.name.clone(),
            focused_tags: state.focused_tags,
            view_tags: state.view_tags.clone(),
            urgent_tags: state.urgent_tags,
            layout_name: state.layout_name.clone(),
        }
    }
}

#[Object(name = "OutputState")]
impl GOutputState {
    async fn output_id(&self) -> &ID {
        &self.output_id
    }

    async fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    async fn focused_tags(&self) -> Option<i32> {
        self.focused_tags
    }

    async fn view_tags(&self) -> Option<&Vec<i32>> {
        self.view_tags.as_ref()
    }

    async fn urgent_tags(&self) -> Option<i32> {
        self.urgent_tags
    }

    async fn layout_name(&self) -> Option<&str> {
        self.layout_name.as_deref()
    }
}

impl RiverSnapshot {
    fn update_output_state<F>(
        &mut self,
        object_id: &wayland_backend::client::ObjectId,
        name: &Option<String>,
        f: F,
    ) where
        F: FnOnce(&mut OutputState),
    {
        let output_id = id_to_graphql(object_id);
        let key = output_id.to_string();
        let mut name_clone = name.clone();
        let entry = self
            .outputs
            .entry(key.clone())
            .or_insert_with(|| OutputState {
                output_id: output_id.clone(),
                name: name_clone.clone(),
                focused_tags: None,
                view_tags: None,
                urgent_tags: None,
                layout_name: None,
            });
        entry.output_id = output_id;
        if let Some(name_value) = name_clone.take() {
            if entry.name.as_ref() != Some(&name_value) {
                if let Some(old_name) = &entry.name {
                    self.output_names.remove(old_name);
                }
            }
            self.output_names.insert(name_value.clone(), key);
            entry.name = Some(name_value);
        }
        f(entry);
    }

    pub fn apply_event(&mut self, event: &river::Event) {
        use river::Event::*;
        match event {
            OutputFocusedTags { id, name, tags } => {
                self.update_output_state(id, name, |state| {
                    state.focused_tags = Some(*tags as i32);
                });
            }
            OutputViewTags { id, name, tags } => {
                let converted = tags.iter().map(|v| *v as i32).collect::<Vec<i32>>();
                self.update_output_state(id, name, move |state| {
                    state.view_tags = Some(converted);
                });
            }
            OutputUrgentTags { id, name, tags } => {
                self.update_output_state(id, name, |state| {
                    state.urgent_tags = Some(*tags as i32);
                });
            }
            OutputLayoutName {
                id,
                name: output_name,
                layout,
            } => {
                let layout = layout.clone();
                self.update_output_state(id, output_name, move |state| {
                    state.layout_name = Some(layout);
                });
            }
            OutputLayoutNameClear { id, name } => {
                self.update_output_state(id, name, |state| {
                    state.layout_name = None;
                });
            }
            SeatFocusedOutput { id, name } => {
                self.seat_focused_output = Some(NamedOutputId {
                    output_id: id_to_graphql(id),
                    name: name.clone(),
                });
            }
            SeatUnfocusedOutput { .. } => {
                // ignore this. only store focused output in the snapshot
            }
            SeatFocusedView { title } => {
                self.seat_focused_view = Some(title.clone());
            }
            SeatMode { name } => {
                self.seat_mode = Some(name.clone());
            }
        }
    }

    pub fn output_by_name(&self, name: &str) -> Option<OutputState> {
        if let Some(id_key) = self.output_names.get(name) {
            return self.outputs.get(id_key).cloned();
        }
        self.outputs
            .values()
            .find(|state| state.name.as_deref() == Some(name))
            .cloned()
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
    pub output_id: ID,
    pub name: Option<String>,
    pub tags: i32,
}
#[Object(name = "OutputFocusedTags")]
impl GOutputFocusedTags {
    async fn tags(&self) -> i32 {
        self.tags
    }

    async fn output_id(&self) -> &ID {
        &self.output_id
    }

    async fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

#[derive(Clone)]
pub struct GOutputViewTags {
    pub output_id: ID,
    pub name: Option<String>,
    pub tags: Vec<i32>,
}
#[Object(name = "OutputViewTags")]
impl GOutputViewTags {
    async fn tags(&self) -> &Vec<i32> {
        &self.tags
    }

    async fn output_id(&self) -> &ID {
        &self.output_id
    }

    async fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

#[derive(Clone)]
pub struct GOutputUrgentTags {
    pub output_id: ID,
    pub name: Option<String>,
    pub tags: i32,
}
#[Object(name = "OutputUrgentTags")]
impl GOutputUrgentTags {
    async fn tags(&self) -> i32 {
        self.tags
    }

    async fn output_id(&self) -> &ID {
        &self.output_id
    }

    async fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

#[derive(Clone)]
pub struct GOutputLayoutName {
    pub output_id: ID,
    pub output_name: Option<String>,
    pub layout: String,
}
#[Object(name = "OutputLayoutName")]
impl GOutputLayoutName {
    async fn layout(&self) -> &str {
        &self.layout
    }

    async fn output_id(&self) -> &ID {
        &self.output_id
    }

    async fn output_name(&self) -> Option<&str> {
        self.output_name.as_deref()
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
            OutputFocusedTags {
                id: output_id,
                name,
                tags,
            } => RiverEvent::OutputFocusedTags(GOutputFocusedTags {
                output_id: id_to_graphql(&output_id),
                name,
                tags: tags as i32,
            }),
            OutputViewTags {
                id: output_id,
                name,
                tags,
            } => RiverEvent::OutputViewTags(GOutputViewTags {
                output_id: id_to_graphql(&output_id),
                name,
                tags: tags.into_iter().map(|v| v as i32).collect::<Vec<i32>>(),
            }),
            OutputUrgentTags {
                id: output_id,
                name,
                tags,
            } => RiverEvent::OutputUrgentTags(GOutputUrgentTags {
                output_id: id_to_graphql(&output_id),
                name,
                tags: tags as i32,
            }),
            OutputLayoutName {
                id: output_id,
                name,
                layout,
            } => RiverEvent::OutputLayoutName(GOutputLayoutName {
                output_id: id_to_graphql(&output_id),
                output_name: name,
                layout,
            }),
            OutputLayoutNameClear {
                id: output_id,
                name,
            } => RiverEvent::OutputLayoutName(GOutputLayoutName {
                output_id: id_to_graphql(&output_id),
                output_name: name,
                layout: String::new(),
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

    async fn outputs(&self, ctx: &Context<'_>) -> Vec<GOutputState> {
        let handle = ctx.data_unchecked::<RiverStateHandle>();
        let Ok(snapshot) = handle.read() else {
            return Vec::new();
        };
        snapshot
            .outputs
            .values()
            .cloned()
            .map(GOutputState::from)
            .collect::<Vec<_>>()
    }

    async fn output(&self, ctx: &Context<'_>, name: String) -> Option<GOutputState> {
        let handle = ctx.data_unchecked::<RiverStateHandle>();
        let Ok(snapshot) = handle.read() else {
            return None;
        };
        snapshot.output_by_name(&name).map(GOutputState::from)
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
    async fn events(
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
