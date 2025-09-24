use async_graphql::futures_util::future::ready;
use async_graphql::futures_util::{Stream, StreamExt};
use async_graphql::{Context, EmptyMutation, Enum, ID, Object, Schema, Subscription, Union};
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
#[Object]
impl GOutputFocusedTags {
    async fn tags(&self) -> i32 {
        self.tags
    }
}

#[derive(Clone)]
pub struct GOutputViewTags {
    pub tags: Vec<i32>,
}
#[Object]
impl GOutputViewTags {
    async fn tags(&self) -> &Vec<i32> {
        &self.tags
    }
}

#[derive(Clone)]
pub struct GOutputUrgentTags {
    pub tags: i32,
}
#[Object]
impl GOutputUrgentTags {
    async fn tags(&self) -> i32 {
        self.tags
    }
}

#[derive(Clone)]
pub struct GOutputLayoutName {
    pub name: String,
}
#[Object]
impl GOutputLayoutName {
    async fn name(&self) -> &str {
        &self.name
    }
}

// no-op clear event omitted in minimal schema

#[derive(Clone)]
pub struct GSeatFocusedOutput {
    pub output_id: ID,
}
#[Object]
impl GSeatFocusedOutput {
    async fn output_id(&self) -> &ID {
        &self.output_id
    }
}

#[derive(Clone)]
pub struct GSeatUnfocusedOutput {
    pub output_id: ID,
}
#[Object]
impl GSeatUnfocusedOutput {
    async fn output_id(&self) -> &ID {
        &self.output_id
    }
}

#[derive(Clone)]
pub struct GSeatFocusedView {
    pub title: String,
}
#[Object]
impl GSeatFocusedView {
    async fn title(&self) -> &str {
        &self.title
    }
}

#[derive(Clone)]
pub struct GSeatMode {
    pub name: String,
}
#[Object]
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
            SeatFocusedOutput { id: output_id } => {
                RiverEvent::SeatFocusedOutput(GSeatFocusedOutput {
                    output_id: id_to_graphql(&output_id),
                })
            }
            SeatUnfocusedOutput { id: output_id } => {
                RiverEvent::SeatUnfocusedOutput(GSeatUnfocusedOutput {
                    output_id: id_to_graphql(&output_id),
                })
            }
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
