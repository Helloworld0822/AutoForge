use crate::app::App;
use crate::domain::{PipelineState, ProjectDetailView, ProjectView};
use crate::services::project_watch::hash_json;
use actix::prelude::*;
use actix_web::http::header;
use actix_web::{web, Error, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(60);

fn ws_authorized(req: &HttpRequest, expected: &Option<String>) -> bool {
    let Some(expected) = expected else {
        return true;
    };

    if let Some(header) = req.headers().get(header::AUTHORIZATION) {
        if let Ok(value) = header.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                if token.trim() == expected.as_str() {
                    return true;
                }
            }
        }
    }

    req.query_string().split('&').any(|pair| {
        pair.strip_prefix("access_token=")
            .is_some_and(|token| token == expected)
    })
}

fn unauthorized() -> HttpResponse {
    HttpResponse::Unauthorized().json(json!({ "error": "unauthorized" }))
}

pub async fn projects_websocket(
    req: HttpRequest,
    stream: web::Payload,
    app: web::Data<Arc<App>>,
) -> Result<HttpResponse, Error> {
    if !ws_authorized(&req, &app.config.api_key) {
        return Ok(unauthorized());
    }

    let app = app.get_ref().clone();
    let session = ProjectsWsSession::new(app);
    ws::start(session, &req, stream)
}

pub async fn project_websocket(
    req: HttpRequest,
    stream: web::Payload,
    path: web::Path<Uuid>,
    app: web::Data<Arc<App>>,
) -> Result<HttpResponse, Error> {
    if !ws_authorized(&req, &app.config.api_key) {
        return Ok(unauthorized());
    }

    let project_id = path.into_inner();
    if app.get_project(project_id).await.is_none() {
        return Ok(HttpResponse::NotFound().json(json!({ "error": "project not found" })));
    }

    let app = app.get_ref().clone();
    let session = ProjectWsSession::new(project_id, app);
    ws::start(session, &req, stream)
}

struct ProjectsWsSession {
    app: Arc<App>,
    heartbeat: Instant,
    last_hash: Option<u64>,
}

impl ProjectsWsSession {
    fn new(app: Arc<App>) -> Self {
        Self {
            app,
            heartbeat: Instant::now(),
            last_hash: None,
        }
    }

    fn start_heartbeat(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.heartbeat) > CLIENT_TIMEOUT {
                ctx.stop();
                return;
            }
            ctx.ping(b"");
        });
    }

    fn spawn_watch(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        let app = self.app.clone();
        let addr = ctx.address();
        actix::spawn(async move {
            let mut watch = match app.watch.subscribe_all().await {
                Ok(w) => w,
                Err(_) => return,
            };
            while watch.next_any().await.is_some() {
                addr.do_send(WatchTick);
            }
        });
    }

    fn push_projects(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        let app = self.app.clone();
        let addr = ctx.address();
        actix::spawn(async move {
            let projects = match app.store.list().await {
                Ok(items) => items,
                Err(_) => return,
            };
            let views: Vec<ProjectView> = projects.iter().map(ProjectView::from).collect();
            addr.do_send(PushProjects(views));
        });
    }
}

struct ProjectWsSession {
    project_id: Uuid,
    app: Arc<App>,
    heartbeat: Instant,
    last_hash: Option<u64>,
}

impl ProjectWsSession {
    fn new(project_id: Uuid, app: Arc<App>) -> Self {
        Self {
            project_id,
            app,
            heartbeat: Instant::now(),
            last_hash: None,
        }
    }

    fn start_heartbeat(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.heartbeat) > CLIENT_TIMEOUT {
                ctx.stop();
                return;
            }
            ctx.ping(b"");
        });
    }

    fn spawn_watch(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        let app = self.app.clone();
        let project_id = self.project_id;
        let addr = ctx.address();
        actix::spawn(async move {
            let mut watch = match app.watch.subscribe(project_id).await {
                Ok(w) => w,
                Err(_) => return,
            };
            while watch.next_for(project_id).await.is_some() {
                addr.do_send(WatchTick);
            }
        });
    }

    fn push_project(&mut self, ctx: &mut ws::WebsocketContext<Self>) {
        let app = self.app.clone();
        let project_id = self.project_id;
        let addr = ctx.address();
        actix::spawn(async move {
            let Some(project) = app.get_project(project_id).await else {
                return;
            };
            let view = ProjectDetailView::from(&project);
            addr.do_send(PushProject {
                view,
                terminal: matches!(
                    project.state,
                    PipelineState::Completed | PipelineState::Failed | PipelineState::Cancelled
                ),
            });
        });
    }
}

#[derive(Message)]
#[rtype(result = "()")]
struct WatchTick;

#[derive(Message)]
#[rtype(result = "()")]
struct PushProjects(Vec<ProjectView>);

#[derive(Message)]
#[rtype(result = "()")]
struct PushProject {
    view: ProjectDetailView,
    terminal: bool,
}

impl Actor for ProjectsWsSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.start_heartbeat(ctx);
        self.spawn_watch(ctx);
        self.push_projects(ctx);
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ProjectsWsSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(bytes)) => {
                self.heartbeat = Instant::now();
                ctx.pong(&bytes);
            }
            Ok(ws::Message::Pong(_)) => self.heartbeat = Instant::now(),
            Ok(ws::Message::Text(text)) => {
                self.heartbeat = Instant::now();
                if text.trim() == r#"{"type":"refresh"}"# {
                    self.push_projects(ctx);
                }
            }
            Ok(ws::Message::Close(_)) => ctx.stop(),
            _ => {}
        }
    }
}

impl Handler<WatchTick> for ProjectsWsSession {
    type Result = ();

    fn handle(&mut self, _: WatchTick, ctx: &mut Self::Context) {
        self.push_projects(ctx);
    }
}

impl Handler<PushProjects> for ProjectsWsSession {
    type Result = ();

    fn handle(&mut self, msg: PushProjects, ctx: &mut Self::Context) {
        let payload = json!({ "type": "projects", "data": msg.0 });
        let hash = hash_json(&payload);
        if self.last_hash == Some(hash) {
            return;
        }
        self.last_hash = Some(hash);
        if let Ok(text) = serde_json::to_string(&payload) {
            ctx.text(text);
        }
    }
}

impl Actor for ProjectWsSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.start_heartbeat(ctx);
        self.spawn_watch(ctx);
        self.push_project(ctx);
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ProjectWsSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(bytes)) => {
                self.heartbeat = Instant::now();
                ctx.pong(&bytes);
            }
            Ok(ws::Message::Pong(_)) => self.heartbeat = Instant::now(),
            Ok(ws::Message::Text(text)) => {
                self.heartbeat = Instant::now();
                if text.trim() == r#"{"type":"refresh"}"# {
                    self.push_project(ctx);
                }
            }
            Ok(ws::Message::Close(_)) => ctx.stop(),
            _ => {}
        }
    }
}

impl Handler<WatchTick> for ProjectWsSession {
    type Result = ();

    fn handle(&mut self, _: WatchTick, ctx: &mut Self::Context) {
        self.push_project(ctx);
    }
}

impl Handler<PushProject> for ProjectWsSession {
    type Result = ();

    fn handle(&mut self, msg: PushProject, ctx: &mut Self::Context) {
        let payload = json!({ "type": "project", "data": msg.view });
        let hash = hash_json(&payload);
        if self.last_hash == Some(hash) {
            return;
        }
        self.last_hash = Some(hash);
        if let Ok(text) = serde_json::to_string(&payload) {
            ctx.text(text);
        }
        if msg.terminal {
            ctx.close(Some(ws::CloseCode::Normal.into()));
        }
    }
}
