//! The closed set of session operations against the Docker Engine API.
//!
//! The controller records every network, volume, container, and exec id it creates and acts only on those; an
//! operation on a session it did not create in this run is refused. `cleanup` alone looks beyond the records: it
//! removes every resource that carries the session label, which is how a crashed run's orphans are reclaimed. Until
//! then, the sessions an earlier run left count toward the session limit, since their resources still exist.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bollard::Docker;
use bollard::container::{AttachContainerResults, LogOutput};
use bollard::errors::Error as DockerError;
use bollard::exec::{StartExecOptions, StartExecResults};
use bollard::models::{ContainerState, ContainerStateStatusEnum, ContainerStatsResponse, HostConfig};
use bollard::query_parameters::{
    AttachContainerOptionsBuilder, CreateContainerOptions, KillContainerOptions, ListContainersOptions,
    ListNetworksOptions, ListVolumesOptions, RemoveContainerOptions, RemoveVolumeOptions, StatsOptions,
};
use futures_util::future::join_all;
use futures_util::{Stream, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::request::SessionId;
use crate::template::{self, Exec, Fixed, LABEL_INSTANCE, LABEL_ROLE, LABEL_SESSION, Role, Spec, Vol};

const TERMINATE_WAIT: Duration = Duration::from_secs(5);
/// How long a session's proxy may take to accept its upstream and listen.
const PROXY_READY_WITHIN: Duration = Duration::from_secs(10);
/// Egress checks at once (see [`Controller::check_egress`]), what earlier ones could not remove counted in; more are
/// refused until one ends, or a cleanup removes what they left.
const MAX_CHECKS: usize = 4;
/// How long a started check's container may take to answer: it reads its upstream within 5 s and checks it within 10 s.
const CHECK_WITHIN: Duration = Duration::from_secs(30);
/// The longest answer a check's container may give, and the longest detail in it.
const MAX_CHECK_ANSWER: usize = 1024;
const MAX_CHECK_DETAIL: usize = 200;
/// Why a check may fail, as the proxy's check mode names it.
const CHECK_REASONS: [&str; 7] = ["unreachable", "timeout", "auth", "refused", "tls", "protocol", "unexpected"];

pub type ExecOutput = Pin<Box<dyn Stream<Item = Result<LogOutput, DockerError>> + Send>>;
pub type ExecInput = Pin<Box<dyn AsyncWrite + Send>>;

/// What a session owns, in creation order.
struct Session {
    spec: Spec,
    network: Option<String>,
    volumes: Vec<String>,
    containers: Vec<(Role, String)>,
    /// The agent container's one exec, once attached.
    exec: Option<String>,
}

impl Session {
    fn new(spec: Spec) -> Self {
        Self { spec, network: None, volumes: Vec::new(), containers: Vec::new(), exec: None }
    }

    fn container(&self, role: Role) -> Result<&str, String> {
        self.recorded(role).ok_or_else(|| format!("the session has no {} container", role.name()))
    }

    fn recorded(&self, role: Role) -> Option<&str> {
        self.containers.iter().find(|(r, _)| *r == role).map(|(_, id)| id.as_str())
    }
}

type Shared = Arc<tokio::sync::Mutex<Session>>;

pub struct Controller {
    docker: Docker,
    fixed: Fixed,
    sessions: Mutex<HashMap<SessionId, Shared>>,
    /// Creations hold it shared, cleanup exclusively: a cleanup never reports success while a creation is still
    /// building resources it would miss.
    builds: tokio::sync::RwLock<()>,
    /// When each held session's creation began, outside the session locks, for the walled garden's reaper.
    born: Mutex<HashMap<SessionId, std::time::Instant>>,
    /// The sessions an earlier run left, by their label, which this run holds no record of: each counts toward the
    /// limit until a stop reclaims it or a cleanup removes it.
    inherited: Mutex<HashSet<String>>,
    /// The most sessions at once (`CONTROLLER_MAX_SESSIONS`), held or inherited, whatever the admin asks.
    max_sessions: usize,
    /// The egress checks under way and those whose resources may remain, which together stay within [`MAX_CHECKS`].
    checks: Mutex<Checks>,
}

/// The egress checks' ids, which no session may take: those under way; those whose removal failed, which count until a
/// cleanup removes what they left; and those with a creation Docker never answered, which may still take effect at any
/// time and so count for the rest of this run (the next run's start removes what they made).
#[derive(Default)]
struct Checks {
    running: HashSet<SessionId>,
    left: HashSet<SessionId>,
    uncertain: HashSet<SessionId>,
}

impl Checks {
    fn holds(&self, id: &SessionId) -> bool {
        self.running.contains(id) || self.left.contains(id) || self.uncertain.contains(id)
    }

    fn count(&self) -> usize {
        self.running.len() + self.left.len() + self.uncertain.len()
    }
}

/// What a check may have left once it ends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Left {
    Nothing,
    /// Its removal failed.
    Resources,
    /// A creation of it got no answer from Docker.
    Uncertain,
}

/// A check under way, its id taken until dropped: then given back, or kept as `left` says.
struct Checking<'a> {
    checks: &'a Mutex<Checks>,
    id: SessionId,
    left: Left,
}

impl Drop for Checking<'_> {
    fn drop(&mut self) {
        let mut checks = lock(self.checks);
        checks.running.remove(&self.id);
        match self.left {
            Left::Nothing => {}
            Left::Resources => drop(checks.left.insert(self.id.clone())),
            Left::Uncertain => drop(checks.uncertain.insert(self.id.clone())),
        }
    }
}

/// Why a proxy or check given its upstream said nothing that counts.
#[derive(Debug)]
enum Unheard {
    /// Its standard input took no upstream.
    NotGiven,
    /// It said more than it may without saying it.
    TooLong,
    /// Its output ended first.
    Ended,
}

/// Gives a started proxy or check its upstream as one line on standard input, then closes it: credentials go there
/// only, never in the environment, which `docker inspect` shows. Then reads what it says, up to `limit` bytes, until
/// `heard` makes something of it.
async fn give_and_hear<R>(
    attached: AttachContainerResults,
    upstream: &egress::Upstream,
    limit: usize,
    heard: impl Fn(&[u8]) -> Option<R>,
) -> Result<R, Unheard> {
    let (mut output, mut input) = (attached.output, attached.input);
    let mut line = serde_json::to_vec(upstream).map_err(|_| Unheard::NotGiven)?;
    line.push(b'\n');
    let given = async {
        input.write_all(&line).await?;
        input.flush().await?;
        input.shutdown().await
    };
    given.await.map_err(|_| Unheard::NotGiven)?;
    let mut said = Vec::new();
    while let Some(Ok(chunk)) = output.next().await {
        if let LogOutput::StdOut { message } = chunk {
            said.extend_from_slice(&message);
            if let Some(answer) = heard(&said) {
                return Ok(answer);
            }
            if said.len() > limit {
                return Err(Unheard::TooLong);
            }
        }
    }
    Err(Unheard::Ended)
}

/// What a check's container answered, held to the shape the proxy's check mode prints:
/// `{"ok":true,"latency_ms":N,"ip":IP,"country":CC}` (an IP address, two capitals or digits), or
/// `{"ok":false,"reason":R,"detail":D}` with a known reason and a short printable detail. Anything else is refused.
fn check_outcome(line: &[u8]) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Outcome {
        ok: bool,
        #[serde(default)]
        latency_ms: Option<u64>,
        #[serde(default)]
        ip: Option<std::net::IpAddr>,
        #[serde(default)]
        country: Option<String>,
        #[serde(default)]
        region: Option<String>,
        #[serde(default)]
        city: Option<String>,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        detail: Option<String>,
    }
    let malformed = || "the check's answer is malformed".to_owned();
    let outcome: Outcome = serde_json::from_slice(line).map_err(|_| malformed())?;
    let printable =
        |d: &str| (1..=MAX_CHECK_DETAIL).contains(&d.len()) && d.bytes().all(|b| (b' '..=b'~').contains(&b));
    let country_ok = |c: &str| c.len() == 2 && c.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    match outcome {
        Outcome {
            ok: true,
            latency_ms: Some(ms),
            ip: Some(ip),
            country: Some(country),
            region,
            city,
            reason: None,
            detail: None,
        } if ms <= CHECK_WITHIN.as_millis() as u64
            && country_ok(&country)
            && [&region, &city].iter().all(|p| p.as_deref().is_none_or(egress::place)) =>
        {
            let mut passed = json!({"ok": true, "latency_ms": ms, "ip": ip.to_string(), "country": country});
            for (key, value) in [("region", region), ("city", city)] {
                if let Some(value) = value {
                    passed[key] = json!(value);
                }
            }
            Ok(passed)
        }
        Outcome {
            ok: false,
            latency_ms: None,
            ip: None,
            country: None,
            region: None,
            city: None,
            reason: Some(reason),
            detail: Some(detail),
        } if CHECK_REASONS.contains(&reason.as_str()) && printable(&detail) => {
            Ok(json!({"ok": false, "reason": reason, "detail": detail}))
        }
        _ => Err(malformed()),
    }
}

/// A refused or failed operation: the message the client sees.
pub type OpResult = Result<Value, String>;

fn docker_error(what: &str, e: DockerError) -> String {
    match e {
        DockerError::DockerResponseServerError { status_code, message } => format!("{what}: {status_code} {message}"),
        other => format!("{what}: {other}"),
    }
}

fn status_of(e: &DockerError) -> Option<u16> {
    match e {
        DockerError::DockerResponseServerError { status_code, .. } => Some(*status_code),
        _ => None,
    }
}

/// Removes a container whether or not it runs. A session's containers have no anonymous volumes; its named ones are
/// removed by name.
const FORCE: RemoveContainerOptions = RemoveContainerOptions { force: true, v: false, link: false };

/// What [`Controller::remove_each`] removes.
#[derive(Clone, Copy)]
enum Resource {
    Container,
    Network,
    Volume,
}

/// Files each removal's outcome by name: among the removed when it worked, else among the errors.
fn tally(
    outcomes: impl IntoIterator<Item = (String, Result<(), String>)>,
    gone: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    for (name, outcome) in outcomes {
        match outcome {
            Ok(()) => gone.push(name),
            Err(e) => errors.push(e),
        }
    }
}

/// Removing what is already gone succeeds.
fn absent_ok(result: Result<(), DockerError>, what: &str) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        Err(e) if status_of(&e) == Some(404) => Ok(()),
        Err(e) => Err(docker_error(what, e)),
    }
}

/// Docker may discard, with only a warning, a limit the kernel cannot enforce: a CPU period and quota, the memory
/// limit, the swap limit (it then allows unlimited swap), or the pids limit. Such a container would run unbounded, so
/// unless it kept every one of them exactly it never starts (and the caller removes it).
fn kept_limits(wanted: Option<&HostConfig>, kept: Option<&HostConfig>) -> Result<(), &'static str> {
    let (Some(wanted), Some(kept)) = (wanted, kept) else {
        return Err("could not be checked for its limits");
    };
    if (kept.cpu_period, kept.cpu_quota) != (wanted.cpu_period, wanted.cpu_quota) {
        return Err("did not keep its CPU limit");
    }
    if kept.memory != wanted.memory {
        return Err("did not keep its memory limit");
    }
    if kept.memory_swap != wanted.memory_swap {
        return Err("did not keep its swap limit");
    }
    if kept.pids_limit != wanted.pids_limit {
        return Err("did not keep its pids limit");
    }
    Ok(())
}

/// A container's exit code once it runs no process: exited or dead; or created and never started (a restart whose
/// create or start failed), which has none (-1) and must count too, or that agent could never be restarted.
fn ended(state: &ContainerState) -> Option<i64> {
    if state.running != Some(false) || state.pid != Some(0) {
        return None;
    }
    match state.status? {
        ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::DEAD => Some(state.exit_code.unwrap_or(-1)),
        ContainerStateStatusEnum::CREATED => Some(-1),
        _ => None,
    }
}

/// The guarded value, poisoned or not: a panic while another task held it does not stop the controller.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

/// Every session resource of this installation, or one session's: Docker ANDs the label filters, so another
/// instance's are never matched.
fn label_filter(instance: &str, session: Option<&SessionId>) -> HashMap<String, Vec<String>> {
    let session = session.map_or_else(|| LABEL_SESSION.to_owned(), |id| format!("{LABEL_SESSION}={id}"));
    HashMap::from([("label".to_owned(), vec![format!("{LABEL_INSTANCE}={instance}"), session])])
}

/// CPU% (100 = one CPU) and memory from a non-streamed stats sample, which carries the previous sample too.
fn usage(stats: &ContainerStatsResponse) -> Value {
    let total = |s: Option<&bollard::models::ContainerCpuStats>| {
        (s.and_then(|c| c.cpu_usage.as_ref()).and_then(|u| u.total_usage), s.and_then(|c| c.system_cpu_usage))
    };
    let (cpu_now, sys_now) = total(stats.cpu_stats.as_ref());
    let (cpu_pre, sys_pre) = total(stats.precpu_stats.as_ref());
    let online = stats.cpu_stats.as_ref().and_then(|c| c.online_cpus).unwrap_or(1).max(1);
    let cpu_percent = match (cpu_now, cpu_pre, sys_now, sys_pre) {
        (Some(c), Some(cp), Some(s), Some(sp)) if s > sp && c >= cp => {
            Some((c - cp) as f64 / (s - sp) as f64 * f64::from(online) * 100.0)
        }
        _ => None,
    };
    let memory = stats.memory_stats.as_ref();
    let inactive = memory.and_then(|m| m.stats.as_ref()).and_then(|s| s.get("inactive_file").copied()).unwrap_or(0);
    let used = memory.and_then(|m| m.usage).map(|u| u.saturating_sub(inactive));
    json!({
        "cpu_percent": cpu_percent.map(|p| (p * 10.0).round() / 10.0),
        "memory_bytes": used,
        "memory_limit_bytes": memory.and_then(|m| m.limit),
    })
}

impl Controller {
    pub fn new(docker: Docker, fixed: Fixed, max_sessions: usize) -> Self {
        Self {
            docker,
            fixed,
            sessions: Mutex::new(HashMap::new()),
            builds: tokio::sync::RwLock::new(()),
            born: Mutex::new(HashMap::new()),
            inherited: Mutex::new(HashSet::new()),
            max_sessions,
            checks: Mutex::new(Checks::default()),
        }
    }

    /// Records the sessions whose containers an earlier run left, before this run serves; returns how many.
    pub async fn inherit(&self) -> Result<usize, String> {
        let options = ListContainersOptions {
            all: true,
            filters: Some(label_filter(&self.fixed.instance, None)),
            ..Default::default()
        };
        let containers =
            self.docker.list_containers(Some(options)).await.map_err(|e| docker_error("list containers", e))?;
        // An egress check never outlives its operation: what one left (its container, or its network alone) is removed
        // now rather than counted as a session; what cannot be removed counts toward the checks' bound instead.
        let networks = self
            .docker
            .list_networks(Some(ListNetworksOptions { filters: Some(label_filter(&self.fixed.instance, None)) }))
            .await
            .map_err(|e| docker_error("list networks", e))?;
        let role_of = |labels: &HashMap<String, String>| labels.get(LABEL_ROLE).cloned();
        let mut checks: HashSet<String> = networks
            .into_iter()
            .filter_map(|n| n.labels)
            .filter(|labels| role_of(labels).as_deref() == Some(Role::Check.name()))
            .filter_map(|mut labels| labels.remove(LABEL_SESSION))
            .collect();
        let mut sessions = Vec::new();
        for labels in containers.into_iter().filter_map(|c| c.labels) {
            let check = role_of(&labels).as_deref() == Some(Role::Check.name());
            match labels.get(LABEL_SESSION).cloned() {
                Some(id) if check => drop(checks.insert(id)),
                Some(id) => sessions.push(id),
                None => {}
            }
        }
        for id in checks {
            let Ok(id) = id.parse::<SessionId>() else { continue };
            if let Err(why) = self.remove_labelled(label_filter(&self.fixed.instance, Some(&id))).await {
                tracing::warn!(check = %id, %why, "what an earlier run's egress check left was not removed");
                lock(&self.checks).left.insert(id);
            }
        }
        let left: HashSet<String> = sessions.into_iter().collect();
        let count = left.len();
        *lock(&self.inherited) = left;
        Ok(count)
    }

    /// Whether this is the demo's walled garden: sessions reach the fixture alone.
    fn walled(&self) -> bool {
        self.fixed.bench.as_ref().is_some_and(|b| b.fixture_only)
    }

    fn session(&self, id: &SessionId) -> Result<Shared, String> {
        let sessions = lock(&self.sessions);
        sessions.get(id).cloned().ok_or_else(|| "unknown session".to_owned())
    }

    fn forget(&self, id: &SessionId) {
        lock(&self.sessions).remove(id);
        lock(&self.born).remove(id);
    }

    pub async fn create_session(&self, mut spec: Spec) -> OpResult {
        if spec.walled && !self.walled() {
            return Err("refused: this session needs the walled garden, and this controller does not enforce it".into());
        }
        let _building = self.builds.read().await;
        if let Some(bench) = &self.fixed.bench {
            let spki = std::fs::read_to_string(&bench.spki_file).map_err(|e| format!("bench fixture SPKI: {e}"))?;
            let spki = spki.trim();
            if !template::valid_spki(spki) {
                return Err("the bench fixture SPKI is not a base64 SHA-256".into());
            }
            spec.bench_spki = Some(spki.to_owned());
        }
        let id = spec.session.clone();
        let mut session = {
            let mut sessions = lock(&self.sessions);
            let inherited = lock(&self.inherited);
            if sessions.contains_key(&id) || inherited.contains(&id.to_string()) || lock(&self.checks).holds(&id) {
                return Err("the session exists".into());
            }
            // A record whose rollback was incomplete keeps counting, as an earlier run's session does: its resources
            // still exist.
            let max = self.max_sessions;
            if sessions.len() + inherited.len() >= max {
                return Err(format!("refused: the controller already holds its maximum of {max} sessions"));
            }
            let shared = Arc::new(tokio::sync::Mutex::new(Session::new(spec)));
            let guard = shared.clone().try_lock_owned().map_err(|_| "the session is busy".to_owned())?;
            sessions.insert(id.clone(), shared);
            lock(&self.born).insert(id.clone(), std::time::Instant::now());
            guard
        };
        match self.build(&mut session).await {
            Ok(()) => {
                let containers: serde_json::Map<String, Value> =
                    session.containers.iter().map(|(role, cid)| (role.name().to_owned(), json!(cid))).collect();
                tracing::info!(session = %id, "session created");
                Ok(json!({ "ok": true, "containers": containers }))
            }
            Err(why) => {
                let leftover = self.teardown(&mut session).await;
                if leftover.is_empty() {
                    drop(session);
                    self.forget(&id);
                    tracing::warn!(session = %id, %why, "session creation failed and was rolled back");
                    Err(format!("{why} (rolled back)"))
                } else {
                    tracing::error!(session = %id, %why, ?leftover, "session creation failed; rollback incomplete");
                    Err(format!("{why}; rollback incomplete: {}", leftover.join("; ")))
                }
            }
        }
    }

    /// The network, then the volumes at once; the display, proxy, and browser at once, then the agent, which joins the
    /// browser's namespaces; then they start in that order. Whatever is created is recorded, whatever fails after.
    async fn build(&self, session: &mut Session) -> Result<(), String> {
        let id = session.spec.session.clone();
        let created = self
            .docker
            .create_network(template::network_body(&self.fixed.instance, &id, self.walled()))
            .await
            .map_err(|e| docker_error("create network", e))?;
        session.network = Some(created.id);
        let volumes =
            Vol::ORDER.map(|vol| self.docker.create_volume(template::volume_body(&self.fixed.instance, &id, vol)));
        let mut created = Ok(());
        for result in join_all(volumes).await {
            match result {
                Ok(volume) => session.volumes.push(volume.name),
                Err(e) => created = created.and(Err(docker_error("create volume", e))),
            }
        }
        created?;
        let spec = &session.spec;
        let first = Role::ORDER.into_iter().filter(|role| *role != Role::Agent).map(|role| async move {
            let mut recorded = Vec::new();
            let result = self.create_container(spec, role, None, &mut recorded, None).await;
            (recorded, result.map(drop))
        });
        let mut created = Ok(());
        for (recorded, result) in join_all(first).await {
            session.containers.extend(recorded);
            created = created.and(result);
        }
        created?;
        let browser = session.container(Role::Browser)?.to_owned();
        self.create_container(&session.spec, Role::Agent, Some(&browser), &mut session.containers, None).await?;
        let (agent, first): (Vec<_>, Vec<_>) = session.containers.iter().partition(|(role, _)| *role == Role::Agent);
        for batch in [first, agent] {
            let starts = batch.into_iter().map(|(role, cid)| self.start(&session.spec, *role, cid));
            join_all(starts).await.into_iter().collect::<Result<Vec<()>, String>>()?;
        }
        Ok(())
    }

    async fn start(&self, spec: &Spec, role: Role, cid: &str) -> Result<(), String> {
        if let (Role::Proxy, Some(upstream)) = (role, &spec.egress) {
            return self.start_proxy_with(cid, upstream).await;
        }
        self.docker.start_container(cid, None).await.map_err(|e| docker_error(&format!("start {}", role.name()), e))
    }

    /// Starts the session's proxy and gives it its upstream on standard input (never the environment, which
    /// `docker inspect` shows), then waits until the proxy says it accepted it and listens: until then the session is
    /// not created, and a proxy that refuses its upstream fails the creation instead of leaving a session without one.
    async fn start_proxy_with(&self, cid: &str, upstream: &egress::Upstream) -> Result<(), String> {
        let attached = self.attach_and_start(cid, "proxy").await?;
        let ready = give_and_hear(attached, upstream, 4096, |said| {
            said.windows(14).any(|w| w == b"{\"ready\":true}").then_some(())
        });
        match tokio::time::timeout(PROXY_READY_WITHIN, ready).await {
            Ok(Ok(())) => Ok(()),
            _ => Err("the session's proxy did not accept its upstream".to_owned()),
        }
    }

    /// Attaches to `cid`'s standard input and output, then starts it; `what` names it in errors.
    async fn attach_and_start(&self, cid: &str, what: &str) -> Result<AttachContainerResults, String> {
        let options = AttachContainerOptionsBuilder::new().stdin(true).stdout(true).stream(true).build();
        let attached = (self.docker.attach_container(cid, Some(options)).await)
            .map_err(|e| docker_error(&format!("attach {what}"), e))?;
        self.docker.start_container(cid, None).await.map_err(|e| docker_error(&format!("start {what}"), e))?;
        Ok(attached)
    }

    /// Creates the container of `role` and records it in `containers` at once, so the session's teardown removes it
    /// whatever follows. `unanswered`, if given, is set when Docker gave the creation no answer at all (its client's
    /// timeout): the container may then still appear.
    async fn create_container(
        &self,
        spec: &Spec,
        role: Role,
        browser: Option<&str>,
        containers: &mut Vec<(Role, String)>,
        unanswered: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<String, String> {
        let body = template::container_body(&self.fixed, spec, role, browser)?;
        let wanted = body.host_config.clone();
        let name = template::container_name(&self.fixed.instance, &spec.session, role);
        let options = CreateContainerOptions { name: Some(name), ..Default::default() };
        let created = self.docker.create_container(Some(options), body).await.map_err(|e| {
            if let Some(flag) = unanswered.filter(|_| status_of(&e).is_none()) {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            docker_error(&format!("create {}", role.name()), e)
        })?;
        containers.push((role, created.id.clone()));
        let inspected = self.docker.inspect_container(&created.id, None).await;
        let kept = inspected.ok().and_then(|i| i.host_config);
        kept_limits(wanted.as_ref(), kept.as_ref()).map_err(|lost| format!("the {} container {lost}", role.name()))?;
        Ok(created.id)
    }

    /// Removes everything the session recorded: the agent, which runs in the browser's namespaces, then the other
    /// containers at once; once none is left, the networks and, at once, the volumes. Returns what could not be
    /// removed, which stays recorded.
    async fn teardown(&self, session: &mut Session) -> Vec<String> {
        let mut errors = Vec::new();
        let (agent, rest): (Vec<_>, Vec<_>) =
            std::mem::take(&mut session.containers).into_iter().partition(|(role, _)| *role == Role::Agent);
        for batch in [agent, rest] {
            if !session.containers.is_empty() {
                session.containers.extend(batch);
                continue;
            }
            for ((role, cid), outcome) in self.remove_each(Resource::Container, batch, |(_, cid)| cid).await {
                if let Err(e) = outcome {
                    errors.push(format!("{}: {e}", role.name()));
                    session.containers.push((role, cid));
                }
            }
        }
        if !session.containers.is_empty() {
            return errors;
        }
        session.exec = None;
        if let Some(nid) = session.network.take()
            && let Err(e) = absent_ok(self.docker.remove_network(&nid).await, "remove network")
        {
            errors.push(e);
            session.network = Some(nid);
        }
        let volumes = std::mem::take(&mut session.volumes);
        for (name, outcome) in self.remove_each(Resource::Volume, volumes, |name| name).await {
            if let Err(e) = outcome {
                errors.push(e);
                session.volumes.push(name);
            }
        }
        errors
    }

    /// Removes every item's resource at once, `id` naming it, and returns each item with its outcome; an already absent
    /// resource counts as removed.
    async fn remove_each<T>(
        &self,
        resource: Resource,
        batch: Vec<T>,
        id: impl Fn(&T) -> &str,
    ) -> Vec<(T, Result<(), String>)> {
        let removals = batch.iter().map(|item| {
            let id = id(item);
            async move {
                match resource {
                    Resource::Container => (self.docker.remove_container(id, Some(FORCE)).await, "remove container"),
                    Resource::Network => (self.docker.remove_network(id).await, "remove network"),
                    Resource::Volume => {
                        let options = RemoveVolumeOptions { force: false };
                        (self.docker.remove_volume(id, Some(options)).await, "remove volume")
                    }
                }
            }
        });
        let results = join_all(removals).await;
        batch.into_iter().zip(results).map(|(item, (result, what))| (item, absent_ok(result, what))).collect()
    }

    pub async fn stop_session(&self, id: &SessionId) -> OpResult {
        let Ok(shared) = self.session(id) else { return self.reclaim(id).await };
        let mut session = shared.lock().await;
        let errors = self.teardown(&mut session).await;
        if !errors.is_empty() {
            return Err(format!("stop incomplete: {}", errors.join("; ")));
        }
        drop(session);
        self.forget(id);
        tracing::info!(session = %id, "session stopped and removed");
        Ok(json!({ "ok": true }))
    }

    pub async fn inspect_session(&self, id: &SessionId) -> OpResult {
        let shared = self.session(id)?;
        // Probing takes a second or two; the session is not held meanwhile, so a terminate, an attach, a restart, or
        // a stop is never delayed by it. A container removed meanwhile shows as an inspection error.
        let (containers, devtools) = {
            let session = shared.lock().await;
            (session.containers.clone(), session.spec.devtools)
        };
        let probes = containers.iter().map(|(role, cid)| async move {
            let inspected = self.docker.inspect_container(cid, None).await;
            let state = match inspected {
                Ok(inspected) => inspected.state.unwrap_or_default(),
                Err(e) => return (*role, json!({ "id": cid, "error": docker_error("inspect", e) })),
            };
            let mut entry = json!({
                "id": cid,
                "status": state.status.map(|s| s.to_string()).unwrap_or_default(),
                "running": state.running.unwrap_or(false),
                "exit_code": state.exit_code,
                "oom_killed": state.oom_killed.unwrap_or(false),
            });
            if state.running == Some(true) {
                let options = StatsOptions { stream: false, one_shot: false };
                if let Some(Ok(stats)) = self.docker.stats(cid, Some(options)).next().await
                    && let (Some(map), Value::Object(extra)) = (entry.as_object_mut(), usage(&stats))
                {
                    map.extend(extra);
                }
            }
            (*role, entry)
        });
        let entries = futures_util::future::join_all(probes).await;
        let containers: serde_json::Map<String, Value> =
            entries.into_iter().map(|(role, entry)| (role.name().to_owned(), entry)).collect();
        Ok(json!({ "ok": true, "devtools": devtools, "containers": containers }))
    }

    /// Creates and starts the agent container's one exec, `agent-mcp` as the agent's user, with stdin and stdout.
    pub async fn agent_attach(&self, id: &SessionId) -> Result<(String, ExecOutput, ExecInput), String> {
        let shared = self.session(id)?;
        let mut session = shared.lock().await;
        if session.exec.is_some() {
            return Err("the agent already has its exec; terminate and restart it first".into());
        }
        let exec = self.create_exec(&session, Exec::Agent).await?;
        session.exec = Some(exec.clone());
        let (output, input) = self.start_exec(&exec).await?;
        tracing::info!(session = %id, exec = %exec, "agent exec attached");
        Ok((exec, output, input))
    }

    /// Creates and starts one launcher profile command in the browser container (see [`Exec`]), with its standard
    /// streams, in a session started from a saved profile only.
    pub async fn profile_exec(&self, id: &SessionId, exec: Exec) -> Result<(String, ExecOutput, ExecInput), String> {
        let shared = self.session(id)?;
        let session = shared.lock().await;
        let exec_id = self.create_exec(&session, exec).await?;
        let (output, input) = self.start_exec(&exec_id).await?;
        tracing::info!(session = %id, exec = %exec_id, command = ?exec.command(), "profile exec attached");
        Ok((exec_id, output, input))
    }

    /// Creates `exec` from its template, in its running container, and returns its id.
    async fn create_exec(&self, session: &Session, exec: Exec) -> Result<String, String> {
        let options = template::exec_options(&session.spec, exec)?;
        let role = exec.role();
        let container = session.container(role)?.to_owned();
        let state = self
            .docker
            .inspect_container(&container, None)
            .await
            .map_err(|e| docker_error(&format!("inspect {}", role.name()), e))?;
        if state.state.and_then(|s| s.running) != Some(true) {
            return Err(format!("the {} is not running", role.name()));
        }
        Ok(self.docker.create_exec(&container, options).await.map_err(|e| docker_error("create exec", e))?.id)
    }

    /// Starts a created exec, attached to its standard streams.
    async fn start_exec(&self, exec: &str) -> Result<(ExecOutput, ExecInput), String> {
        let started = self
            .docker
            .start_exec(exec, Some(StartExecOptions { detach: false, tty: false, output_capacity: None }))
            .await
            .map_err(|e| docker_error("start exec", e))?;
        match started {
            StartExecResults::Attached { output, input } => Ok((output, input)),
            StartExecResults::Detached => Err("the exec started detached".into()),
        }
    }

    /// Kills the agent container and answers only with proof that it, and its exec, no longer run. A session without
    /// an agent (a restart removed it and failed to create the fresh one) has nothing running: that is the proof.
    pub async fn agent_terminate(&self, id: &SessionId) -> OpResult {
        let shared = self.session(id)?;
        let session = shared.lock().await;
        let Some(agent) = session.recorded(Role::Agent).map(str::to_owned) else {
            tracing::info!(session = %id, "no agent to terminate");
            return Ok(json!({ "ok": true, "exited": true, "exit_code": null }));
        };
        let kill = KillContainerOptions { signal: "SIGKILL".to_owned() };
        match self.docker.kill_container(&agent, Some(kill)).await {
            Ok(()) => {}
            // 409: the container is not running, which the inspection below must still prove.
            Err(e) if status_of(&e) == Some(409) => {}
            Err(e) => return Err(docker_error("kill agent", e)),
        }
        let deadline = tokio::time::Instant::now() + TERMINATE_WAIT;
        loop {
            let proof = self.terminated(&agent, session.exec.as_deref()).await;
            match proof {
                Ok(Some(exit_code)) => {
                    tracing::info!(session = %id, exit_code, "agent terminated");
                    return Ok(json!({ "ok": true, "exited": true, "exit_code": exit_code }));
                }
                Ok(None) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Ok(None) => {
                    tracing::error!(session = %id, "agent termination not proven");
                    return Err("the agent's termination is not proven".into());
                }
                Err(why) => {
                    tracing::error!(session = %id, %why, "agent termination not proven");
                    return Err(why);
                }
            }
        }
    }

    /// `Some(exit code)` once the container is inspected as having no process (see [`ended`]) and its exec as not
    /// running.
    async fn terminated(&self, agent: &str, exec: Option<&str>) -> Result<Option<i64>, String> {
        let inspected =
            self.docker.inspect_container(agent, None).await.map_err(|e| docker_error("inspect agent", e))?;
        let Some(exit_code) = ended(&inspected.state.unwrap_or_default()) else { return Ok(None) };
        if let Some(exec) = exec {
            let inspected = self.docker.inspect_exec(exec).await.map_err(|e| docker_error("inspect exec", e))?;
            if inspected.running != Some(false) {
                return Ok(None);
            }
        }
        Ok(Some(exit_code))
    }

    /// Replaces a terminated agent container with a fresh one from the template, or creates it when there is none (an
    /// earlier restart removed it and failed to create the fresh one).
    pub async fn agent_restart(&self, id: &SessionId) -> OpResult {
        // It creates a container, so a cleanup must not run meanwhile (see `builds`).
        let _building = self.builds.read().await;
        let shared = self.session(id)?;
        let mut session = shared.lock().await;
        let browser = session.container(Role::Browser)?.to_owned();
        if let Some(agent) = session.recorded(Role::Agent).map(str::to_owned) {
            if self.terminated(&agent, session.exec.as_deref()).await?.is_none() {
                return Err("the agent is not proven terminated; terminate it first".into());
            }
            absent_ok(self.docker.remove_container(&agent, Some(FORCE)).await, "remove agent")?;
            session.containers.retain(|(role, _)| *role != Role::Agent);
        }
        session.exec = None;
        let session = &mut *session;
        let fresh =
            self.create_container(&session.spec, Role::Agent, Some(&browser), &mut session.containers, None).await?;
        self.docker.start_container(&fresh, None).await.map_err(|e| docker_error("start agent", e))?;
        tracing::info!(session = %id, agent = %fresh, "agent restarted");
        Ok(json!({ "ok": true, "agent": fresh }))
    }

    /// Stops every session older than `max`: the walled garden's limit holds even if the admin that should have
    /// stopped them is gone.
    pub async fn reap(&self, max: Duration) {
        let old: Vec<SessionId> = {
            let born = lock(&self.born);
            born.iter().filter(|(_, at)| at.elapsed() > max).map(|(id, _)| id.clone()).collect()
        };
        for id in old {
            match self.stop_session(&id).await {
                Ok(_) => tracing::warn!(session = %id, "stopped a session past the walled garden's limit"),
                Err(why) => tracing::error!(session = %id, %why, "a session past the limit did not stop"),
            }
        }
    }

    /// Checks an upstream from where a session's proxy reaches it: on a network of its own made as a session's is (its
    /// host interface `er-<id>`, so the host rule holds; the fixture network too in the bench profile), one container
    /// from the check template, the proxy image in its check mode, is given the upstream on standard input (never the
    /// environment), and its one line of answer is read back, held to its shape, within [`CHECK_WITHIN`]. Both are
    /// removed before the answer goes back, whatever the outcome; what a removal leaves carries the labels a cleanup
    /// removes. At most [`MAX_CHECKS`] at once, beside the sessions and outside their limit; never in the walled
    /// garden, whose sessions leave through no egress.
    pub async fn check_egress(&self, upstream: egress::Upstream) -> OpResult {
        if self.walled() {
            return Err("refused: the walled garden has no egress to check".into());
        }
        // It creates resources, so a cleanup must not run meanwhile (see `builds`).
        let _building = self.builds.read().await;
        let mut checking = self.check_id()?;
        let spec = Spec::check(checking.id.clone());
        let uncertain = std::sync::atomic::AtomicBool::new(false);
        let outcome = self.run_check(&spec, &upstream, &uncertain).await;
        // Whatever carries its id goes. A creation that got no answer from Docker (its client's timeout) may still
        // finish after this, so its id then stays counted until a cleanup.
        let removed = self.remove_labelled(label_filter(&self.fixed.instance, Some(&checking.id))).await;
        checking.left = if uncertain.load(std::sync::atomic::Ordering::SeqCst) {
            Left::Uncertain
        } else if removed.is_err() {
            Left::Resources
        } else {
            Left::Nothing
        };
        if let Err(why) = removed {
            tracing::error!(check = %checking.id, %why, "an egress check left resources; a cleanup removes them");
        }
        let outcome = outcome?;
        tracing::info!(check = %checking.id, passed = %outcome["ok"], "egress checked");
        Ok(json!({ "ok": true, "outcome": outcome }))
    }

    /// A fresh id for a check, which no session or other check holds, taken until the returned guard is dropped; refused
    /// while [`MAX_CHECKS`] are under way or left.
    fn check_id(&self) -> Result<Checking<'_>, String> {
        loop {
            let mut bytes = [0u8; 6];
            // The kernel's generator; a failure leaves zeros, which the loop then takes only if nothing holds them.
            let _ = rustix::rand::getrandom(&mut bytes, rustix::rand::GetRandomFlags::empty());
            let text: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let Ok(id) = text.parse::<SessionId>() else { continue };
            let held = lock(&self.sessions).contains_key(&id) || lock(&self.inherited).contains(&text);
            let mut checks = lock(&self.checks);
            if checks.count() >= MAX_CHECKS {
                return Err(format!("refused: {MAX_CHECKS} egress checks are under way; try again shortly"));
            }
            if !held && !checks.holds(&id) {
                checks.running.insert(id.clone());
                return Ok(Checking { checks: &self.checks, id, left: Left::Nothing });
            }
        }
    }

    /// The check itself: its resources made (each Docker call bounded by the client's own timeout), then its answer,
    /// within [`CHECK_WITHIN`].
    /// `uncertain` is set when a creation got no answer from Docker.
    async fn run_check(
        &self,
        spec: &Spec,
        upstream: &egress::Upstream,
        uncertain: &std::sync::atomic::AtomicBool,
    ) -> Result<Value, String> {
        let unanswered = |e: &DockerError| status_of(e).is_none();
        let network =
            self.docker.create_network(template::check_network_body(&self.fixed.instance, &spec.session)).await;
        if let Err(e) = network {
            uncertain.store(unanswered(&e), std::sync::atomic::Ordering::SeqCst);
            return Err(docker_error("create network", e));
        }
        let mut recorded = Vec::new();
        let cid = self.create_container(spec, Role::Check, None, &mut recorded, Some(uncertain)).await?;
        let attached = self.attach_and_start(&cid, "check").await?;
        let answered = give_and_hear(attached, upstream, MAX_CHECK_ANSWER, |said| {
            said.iter().position(|&b| b == b'\n').map(|end| check_outcome(&said[..end]))
        });
        match tokio::time::timeout(CHECK_WITHIN, answered).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(Unheard::NotGiven)) => Err("the check did not take its upstream".to_owned()),
            Ok(Err(Unheard::TooLong)) => Err("the check's answer is too long".to_owned()),
            Ok(Err(Unheard::Ended)) => Err("the check ended without an answer".to_owned()),
            Err(_) => Err("the check did not answer in time".to_owned()),
        }
    }

    /// Removes every container, network, and volume labelled with a session of this instance, what a crashed earlier
    /// run left behind included, and forgets every record.
    pub async fn cleanup(&self) -> OpResult {
        let _quiet = self.builds.write().await;
        let removed = self.remove_labelled(label_filter(&self.fixed.instance, None)).await?;
        // Records go only once their resources are confirmed gone.
        lock(&self.sessions).clear();
        lock(&self.born).clear();
        lock(&self.inherited).clear();
        // What a creation Docker never answered made may still appear: those stay counted.
        lock(&self.checks).left.clear();
        Ok(json!({ "ok": true, "removed": removed }))
    }

    /// Removes what carries one session's label, for a session this run does not hold (an earlier run created it).
    /// Nothing labelled is a stopped session too.
    async fn reclaim(&self, id: &SessionId) -> OpResult {
        let removed = self.remove_labelled(label_filter(&self.fixed.instance, Some(id))).await?;
        lock(&self.inherited).remove(&id.to_string());
        tracing::info!(session = %id, %removed, "reclaimed a session this run did not hold");
        Ok(json!({ "ok": true, "reclaimed": removed }))
    }

    /// Removes the containers, networks, and volumes that match a label filter; an already absent one counts as
    /// removed. As in `teardown`, the agents go first, since they run in their browsers' namespaces, then the other
    /// containers at once; then the networks and, at once, the volumes.
    async fn remove_labelled(&self, label: HashMap<String, Vec<String>>) -> Result<Value, String> {
        let filters = Some(label);
        let mut errors = Vec::new();
        let (mut gone_containers, mut gone_networks, mut gone_volumes) = (Vec::new(), Vec::new(), Vec::new());
        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions { all: true, filters: filters.clone(), ..Default::default() }))
            .await
            .map_err(|e| docker_error("list containers", e))?;
        let (agents, rest): (Vec<_>, Vec<_>) = containers
            .into_iter()
            .filter_map(|container| {
                let cid = container.id?;
                let name = container.names.and_then(|n| n.first().cloned()).unwrap_or_else(|| cid.clone());
                let role = container.labels.as_ref().and_then(|labels| labels.get(LABEL_ROLE));
                Some((role.is_some_and(|r| r == Role::Agent.name()), cid, name.trim_start_matches('/').to_owned()))
            })
            .partition(|(agent, ..)| *agent);
        for batch in [agents, rest] {
            let outcomes = self.remove_each(Resource::Container, batch, |(_, cid, _)| cid).await;
            tally(outcomes.into_iter().map(|((.., name), outcome)| (name, outcome)), &mut gone_containers, &mut errors);
        }
        let networks: Vec<_> = self
            .docker
            .list_networks(Some(ListNetworksOptions { filters: filters.clone() }))
            .await
            .map_err(|e| docker_error("list networks", e))?
            .into_iter()
            .filter_map(|network| network.id.map(|nid| (network.name.unwrap_or_else(|| nid.clone()), nid)))
            .collect();
        let outcomes = self.remove_each(Resource::Network, networks, |(_, nid)| nid).await;
        tally(outcomes.into_iter().map(|((name, _), outcome)| (name, outcome)), &mut gone_networks, &mut errors);
        let volumes: Vec<_> = self
            .docker
            .list_volumes(Some(ListVolumesOptions { filters }))
            .await
            .map_err(|e| docker_error("list volumes", e))?
            .volumes
            .into_iter()
            .flatten()
            .map(|volume| volume.name)
            .collect();
        tally(self.remove_each(Resource::Volume, volumes, |name| name).await, &mut gone_volumes, &mut errors);
        let removed = json!({ "containers": gone_containers, "networks": gone_networks, "volumes": gone_volumes });
        tracing::info!(%removed, "removed labelled resources");
        if errors.is_empty() {
            Ok(removed)
        } else {
            Err(format!("removal incomplete: {}; removed {removed}", errors.join("; ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{ContainerCpuStats, ContainerCpuUsage, ContainerMemoryStats};

    /// A controller whose Docker Engine is unreachable (`/dev/null` is no socket), allowed `max_sessions` sessions and
    /// holding one session with a browser and no agent: what a restart that removed the agent and failed to create the
    /// fresh one leaves.
    fn holding_one(max_sessions: usize) -> (Controller, SessionId) {
        let spec = template::tests::spec(false);
        let id = spec.session.clone();
        let docker = Docker::connect_with_unix("/dev/null", 1, bollard::API_DEFAULT_VERSION).unwrap();
        let controller = Controller::new(docker, template::tests::fixed(), max_sessions);
        let mut session = Session::new(spec);
        session.containers.push((Role::Browser, template::tests::BROWSER_ID.into()));
        controller.sessions.lock().unwrap().insert(id.clone(), Arc::new(tokio::sync::Mutex::new(session)));
        (controller, id)
    }

    #[tokio::test]
    async fn a_session_whose_agent_is_gone_has_it_terminated_and_restarted_by_creating_it() {
        let (controller, id) = holding_one(16);
        let terminated = controller.agent_terminate(&id).await.unwrap();
        assert_eq!(terminated["exited"], json!(true), "nothing runs: no Docker call is needed to prove it");
        // The restart goes straight to creating the agent, which this unreachable Docker Engine refuses.
        let why = controller.agent_restart(&id).await.unwrap_err();
        assert!(why.contains("create agent"), "{why}");
    }

    #[tokio::test]
    async fn a_profile_exec_is_refused_before_any_docker_call_in_a_session_without_a_profile() {
        let (controller, id) = holding_one(16);
        let why = controller.profile_exec(&id, Exec::ProfileSave).await.err().unwrap();
        assert!(why.contains("a profile exec in a session without a profile"), "{why}");
        controller.session(&id).unwrap().lock().await.spec.profile = true;
        let why = controller.profile_exec(&id, Exec::ProfileRestore).await.err().unwrap();
        assert!(why.contains("inspect browser"), "with a profile it goes on to Docker (unreachable here): {why}");
        let unknown: SessionId = "ba5eba11ba5e".parse().unwrap();
        assert_eq!(controller.profile_exec(&unknown, Exec::ProfileSave).await.err().unwrap(), "unknown session");
    }

    #[tokio::test]
    async fn a_creation_past_the_session_limit_is_refused_before_any_docker_call() {
        let (controller, held) = holding_one(1);
        let mut spec = controller.session(&held).unwrap().lock().await.spec.clone();
        spec.session = "ba5eba11ba5e".parse().unwrap();
        let why = controller.create_session(spec.clone()).await.unwrap_err();
        assert!(why.contains("maximum of 1 sessions"), "{why}");
        assert_eq!(controller.sessions.lock().unwrap().len(), 1, "the refused session left no record");
        // Below the limit the creation goes on to Docker (unreachable here) and is rolled back.
        let (controller, _) = holding_one(2);
        let why = controller.create_session(spec.clone()).await.unwrap_err();
        assert!(why.contains("create network") && why.ends_with("(rolled back)"), "{why}");
        assert_eq!(controller.sessions.lock().unwrap().len(), 1);
        // A session an earlier run left counts too, and its id is taken.
        controller.inherited.lock().unwrap().insert("cafecafecafe".into());
        let why = controller.create_session(spec.clone()).await.unwrap_err();
        assert!(why.contains("maximum of 2 sessions"), "{why}");
        let (controller, _) = holding_one(3);
        controller.inherited.lock().unwrap().insert(spec.session.to_string());
        assert_eq!(controller.create_session(spec).await.unwrap_err(), "the session exists");
    }

    #[test]
    fn a_check_s_answer_is_held_to_its_shape() {
        let passed = br#"{"ok":true,"latency_ms":412,"ip":"203.0.113.9","country":"BR"}"#;
        assert_eq!(
            check_outcome(passed),
            Ok(json!({"ok": true, "latency_ms": 412, "ip": "203.0.113.9", "country": "BR"}))
        );
        let placed =
            r#"{"ok":true,"latency_ms":412,"ip":"203.0.113.9","country":"BR","region":"São Paulo","city":"Campinas"}"#;
        assert_eq!(check_outcome(placed.as_bytes()).unwrap()["city"], "Campinas");
        let failed = br#"{"ok":false,"reason":"auth","detail":"the proxy refused the user name or password"}"#;
        assert_eq!(check_outcome(failed).unwrap()["reason"], "auth");
        for bad in [
            &br#"{"ok":true,"latency_ms":412,"ip":"203.0.113.9","country":"BR","city":"<script>"}"#[..],
            br#"{"ok":false,"reason":"auth","detail":"x","region":"Paris"}"#,
            &br#"{"ok":true}"#[..],
            br#"{"ok":true,"latency_ms":412}"#,
            br#"{"ok":true,"latency_ms":412,"ip":"203.0.113.9","country":"BR","detail":"x"}"#,
            br#"{"ok":true,"latency_ms":99999999,"ip":"203.0.113.9","country":"BR"}"#,
            br#"{"ok":true,"latency_ms":412,"ip":"somewhere","country":"BR"}"#,
            br#"{"ok":true,"latency_ms":412,"ip":"203.0.113.9","country":"Brazil"}"#,
            br#"{"ok":false,"reason":"other","detail":"x"}"#,
            br#"{"ok":false,"reason":"auth","detail":""}"#,
            br#"{"ok":false,"reason":"auth","detail":"line
break"}"#,
            br#"{"ok":false,"reason":"auth","detail":"x","password":"hunter2"}"#,
            br#"{"ok":false,"reason":"auth"}"#,
            b"not json",
        ] {
            assert!(check_outcome(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        let long = format!(r#"{{"ok":false,"reason":"tls","detail":"{}"}}"#, "a".repeat(MAX_CHECK_DETAIL + 1));
        assert!(check_outcome(long.as_bytes()).is_err());
    }

    #[tokio::test]
    async fn a_check_is_refused_in_the_walled_garden_and_past_its_bound_before_any_docker_call() {
        let upstream = egress::Upstream::parse_url("http://proxy.example.net:8080").unwrap();
        let (controller, held) = holding_one(1);
        // Beside a session at the limit, which a check does not count toward: on to Docker (unreachable here).
        let why = controller.check_egress(upstream.clone()).await.unwrap_err();
        assert!(why.contains("create network"), "{why}");
        {
            let checks = controller.checks.lock().unwrap();
            assert!(checks.running.is_empty(), "its id is given back");
            // Docker never answered its creation: what it may have made counts, past a cleanup too.
            assert_eq!((checks.left.len(), checks.uncertain.len()), (0, 1));
        }
        let held_checks: Vec<_> = (1..MAX_CHECKS).map(|_| controller.check_id().unwrap()).collect();
        let why = controller.check_egress(upstream.clone()).await.unwrap_err();
        assert!(why.contains("egress checks are under way"), "{why}");
        drop(held_checks);
        controller.checks.lock().unwrap().uncertain.clear();
        // An id a check holds is no session's.
        let checking = controller.check_id().unwrap();
        assert_ne!(checking.id, held);
        let mut spec = controller.session(&held).unwrap().lock().await.spec.clone();
        spec.session = checking.id.clone();
        assert_eq!(controller.create_session(spec).await.unwrap_err(), "the session exists");
        drop(checking);
        let mut fixed = template::tests::fixed();
        fixed.bench = Some(template::Bench {
            network: "fixtures".into(),
            fixture: "fixture.bench=10.250.250.10".into(),
            spki_file: "/nonexistent".into(),
            lanes: Vec::new(),
            fixture_only: true,
        });
        let docker = Docker::connect_with_unix("/dev/null", 1, bollard::API_DEFAULT_VERSION).unwrap();
        let walled = Controller::new(docker, fixed, 1);
        let why = walled.check_egress(upstream).await.unwrap_err();
        assert!(why.contains("walled garden"), "{why}");
    }

    #[test]
    fn a_container_that_lost_any_limit_never_starts() {
        let wanted = HostConfig {
            cpu_period: Some(10_000),
            cpu_quota: Some(20_000),
            memory: Some(2048 << 20),
            memory_swap: Some(2048 << 20),
            pids_limit: Some(2048),
            ..Default::default()
        };
        assert_eq!(kept_limits(Some(&wanted), Some(&wanted.clone())), Ok(()));
        // What Docker leaves when the kernel cannot enforce a limit.
        let w = || wanted.clone();
        for (limit, kept) in [
            ("CPU", HostConfig { cpu_quota: None, ..w() }),
            ("CPU", HostConfig { cpu_period: Some(0), ..w() }),
            ("memory", HostConfig { memory: Some(0), ..w() }),
            ("swap", HostConfig { memory_swap: Some(-1), ..w() }),
            ("pids", HostConfig { pids_limit: None, ..w() }),
            ("pids", HostConfig { pids_limit: Some(0), ..w() }),
        ] {
            let why = kept_limits(Some(&wanted), Some(&kept)).unwrap_err();
            assert_eq!(why, format!("did not keep its {limit} limit"), "{kept:?}");
        }
        assert!(kept_limits(Some(&wanted), None).is_err(), "an inspection that failed proves nothing");
        assert!(kept_limits(None, Some(&wanted)).is_err());
    }

    #[test]
    fn only_a_container_without_a_process_has_ended() {
        let state = |status, running, pid, exit_code| ContainerState {
            status: Some(status),
            running: Some(running),
            pid: Some(pid),
            exit_code: Some(exit_code),
            ..Default::default()
        };
        assert_eq!(ended(&state(ContainerStateStatusEnum::EXITED, false, 0, 137)), Some(137));
        assert_eq!(ended(&state(ContainerStateStatusEnum::DEAD, false, 0, 1)), Some(1));
        assert_eq!(ended(&state(ContainerStateStatusEnum::CREATED, false, 0, 0)), Some(-1), "never started");
        assert_eq!(ended(&state(ContainerStateStatusEnum::RUNNING, true, 42, 0)), None);
        assert_eq!(ended(&state(ContainerStateStatusEnum::PAUSED, false, 42, 0)), None);
        assert_eq!(ended(&state(ContainerStateStatusEnum::CREATED, false, 42, 0)), None, "a process runs");
        assert_eq!(ended(&state(ContainerStateStatusEnum::EXITED, true, 0, 0)), None);
        assert_eq!(ended(&ContainerState::default()), None);
    }

    #[test]
    fn cpu_and_memory_from_two_samples() {
        let cpu = |total, system| ContainerCpuStats {
            cpu_usage: Some(ContainerCpuUsage { total_usage: Some(total), ..Default::default() }),
            system_cpu_usage: Some(system),
            online_cpus: Some(4),
            ..Default::default()
        };
        let stats = ContainerStatsResponse {
            cpu_stats: Some(cpu(1_500, 10_000)),
            precpu_stats: Some(cpu(1_000, 8_000)),
            memory_stats: Some(ContainerMemoryStats {
                usage: Some(100),
                limit: Some(1000),
                stats: Some(HashMap::from([("inactive_file".to_owned(), 30)])),
                ..Default::default()
            }),
            ..Default::default()
        };
        let value = usage(&stats);
        assert_eq!(value["cpu_percent"], json!(100.0));
        assert_eq!(value["memory_bytes"], json!(70));
        assert_eq!(value["memory_limit_bytes"], json!(1000));
        assert_eq!(usage(&ContainerStatsResponse::default())["cpu_percent"], Value::Null);
    }
}
