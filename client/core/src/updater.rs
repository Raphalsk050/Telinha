//! Keeps the app up to date: looks for a newer release, downloads it next to the data and, once
//! the app closes, has the engine copy it over the program. Same rules as `desktop/src/updater.js`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

pub const REPOSITORY: &str = "Raphalsk050/Telinha";
const ASSET_NAME: &str = "Telinha.exe";
const USER_AGENT: &str = "Telinha-Updater";
const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_INTERVAL: Duration = Duration::from_secs(60);
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(10 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_BYTES: u64 = 1024 * 1024 * 1024;
const APPLIED_FILE: &str = "applied.txt";
const FAILED_FILE: &str = "failed.txt";
const HELPER_NAME: &str = "telinha-helper.exe";

/// Where the releases are. The tests point these at a server of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// Answers with a redirect to the page of the latest release, whose address ends in its tag.
    pub release_page: String,
    /// Describes the latest release, with its files.
    pub release_api: String,
    /// What the address of a file of a release must start with.
    pub download_prefix: String,
}

impl Endpoints {
    pub fn github() -> Self {
        Self {
            release_page: format!("https://github.com/{REPOSITORY}/releases/latest"),
            release_api: format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
            download_prefix: format!("https://github.com/{REPOSITORY}/releases/download/"),
        }
    }
}

/// `1.2.3` or `v1.2.3`, and nothing else.
pub fn parse_version(text: &str) -> Option<[u64; 3]> {
    let text = text.trim();
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut number = || {
        let part = parts.next()?;
        (!part.is_empty() && part.len() <= 9 && part.bytes().all(|byte| byte.is_ascii_digit())).then(|| part.parse().ok()).flatten()
    };
    let version = [number()?, number()?, number()?];
    parts.next().is_none().then_some(version)
}

fn written(version: [u64; 3]) -> String {
    format!("{}.{}.{}", version[0], version[1], version[2])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<String>,
}

/// The file to download from a release, when the release is newer than the app and looks right.
pub fn pick_update(release: &Value, current: &str, download_prefix: &str) -> Option<Update> {
    let current = parse_version(current)?;
    let version = parse_version(release.get("tag_name")?.as_str()?)?;
    let flag = |name: &str| release.get(name).and_then(Value::as_bool).unwrap_or(false);
    if flag("draft") || flag("prerelease") || version <= current {
        return None;
    }
    let asset = release.get("assets")?.as_array()?.iter().find(|asset| asset.get("name").and_then(Value::as_str) == Some(ASSET_NAME))?;
    let url = asset.get("browser_download_url")?.as_str()?;
    let size = asset.get("size")?.as_u64()?;
    if !url.starts_with(download_prefix) || size == 0 || size > MAX_BYTES {
        return None;
    }
    let sha256 = asset
        .get("digest")
        .and_then(Value::as_str)
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        .map(str::to_owned);
    Some(Update { version: written(version), url: url.to_owned(), size, sha256 })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Downloading,
    /// Downloaded, waiting for the app to close.
    Ready,
    /// Downloaded, but Windows refused the swap the last time.
    Blocked,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateState {
    pub status: UpdateStatus,
    pub version: Option<String>,
    pub progress: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateEvent {
    Changed(UpdateState),
    Failed(String),
}

/// What the engine needs to swap the program after the app closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyRequest {
    pub helper: PathBuf,
    pub source: PathBuf,
    pub target: PathBuf,
    pub version: String,
    /// The swap waits for this process to end.
    pub wait_pid: u32,
    pub applied_file: PathBuf,
    pub failed_file: PathBuf,
    pub relaunch: bool,
    pub elevate: bool,
}

/// Starts the engine in its `apply-update` mode, from a copy next to the download, cut loose from
/// this process so it outlives the app.
pub fn apply_staged(request: &ApplyRequest) -> Result<(), String> {
    let folder = request.source.parent().ok_or("a versão baixada não tem pasta")?;
    let runner = folder.join(HELPER_NAME);
    std::fs::copy(&request.helper, &runner).map_err(|error| format!("não consegui copiar o ajudante: {error}"))?;
    let mut command = std::process::Command::new(&runner);
    command
        .arg("apply-update")
        .current_dir(folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .env("TELINHA_UPDATE_SOURCE", &request.source)
        .env("TELINHA_UPDATE_TARGET", &request.target)
        .env("TELINHA_UPDATE_VERSION", &request.version)
        .env("TELINHA_UPDATE_PID", request.wait_pid.to_string())
        .env("TELINHA_UPDATE_APPLIED", &request.applied_file)
        .env("TELINHA_UPDATE_FAILED", &request.failed_file)
        .env("TELINHA_UPDATE_RELAUNCH", if request.relaunch { "1" } else { "0" })
        .env("TELINHA_UPDATE_ELEVATE", if request.elevate { "1" } else { "0" });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command.spawn().map(|_| ()).map_err(|error| format!("não consegui abrir o ajudante: {error}"))
}

type Listener = Box<dyn Fn(UpdateEvent) + Send + Sync>;
type Apply = Box<dyn Fn(&ApplyRequest) -> Result<(), String> + Send + Sync>;

pub struct Updater {
    current: String,
    target: PathBuf,
    stage_dir: PathBuf,
    helper: PathBuf,
    endpoints: Endpoints,
    /// Follows redirects, for the files. The page of the latest release is asked without.
    client: reqwest::Client,
    page: reqwest::Client,
    state: Mutex<UpdateState>,
    busy: AtomicBool,
    applying: AtomicBool,
    retry_at: Mutex<Option<Instant>>,
    listener: Listener,
    apply: Apply,
}

impl Updater {
    /// `target` is the program to replace, `stage_dir` where the download waits, `helper` the
    /// engine. `listener` hears every change, from any thread.
    pub fn new(current: &str, target: &Path, stage_dir: &Path, helper: &Path, endpoints: Endpoints, listener: impl Fn(UpdateEvent) + Send + Sync + 'static) -> Self {
        let client = |follow: bool| {
            let policy = if follow { reqwest::redirect::Policy::limited(8) } else { reqwest::redirect::Policy::none() };
            reqwest::Client::builder().user_agent(USER_AGENT).redirect(policy).build().unwrap_or_default()
        };
        Self {
            current: current.to_owned(),
            target: target.to_owned(),
            stage_dir: stage_dir.to_owned(),
            helper: helper.to_owned(),
            endpoints,
            client: client(true),
            page: client(false),
            state: Mutex::new(UpdateState { status: UpdateStatus::Idle, version: None, progress: 0.0 }),
            busy: AtomicBool::new(false),
            applying: AtomicBool::new(false),
            retry_at: Mutex::new(None),
            listener: Box::new(listener),
            apply: Box::new(apply_staged),
        }
    }

    /// Swaps what does the swap. For the tests.
    pub fn with_apply(mut self, apply: impl Fn(&ApplyRequest) -> Result<(), String> + Send + Sync + 'static) -> Self {
        self.apply = Box::new(apply);
        self
    }

    pub fn current(&self) -> &str {
        &self.current
    }

    pub fn snapshot(&self) -> UpdateState {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Checks a little after the app opens and then every minute. Must be called from inside a
    /// tokio runtime.
    pub fn start(self: &std::sync::Arc<Self>) {
        let updater = std::sync::Arc::downgrade(self);
        tokio::spawn(async move {
            if let Some(updater) = updater.upgrade() {
                updater.remove_leftovers();
            }
            tokio::time::sleep(FIRST_CHECK).await;
            loop {
                let Some(updater) = updater.upgrade() else { return };
                updater.check().await;
                drop(updater);
                tokio::time::sleep(CHECK_INTERVAL).await;
            }
        });
    }

    fn staged_file(&self, version: &str) -> PathBuf {
        self.stage_dir.join(format!("Telinha-{version}.exe"))
    }

    fn read_mark(&self, name: &str) -> String {
        std::fs::read_to_string(self.stage_dir.join(name)).map(|text| text.trim().to_owned()).unwrap_or_default()
    }

    /// Removes what an interrupted download left, and downloads that are no longer newer than
    /// the app.
    pub fn remove_leftovers(&self) {
        let current = parse_version(&self.current);
        let _ = std::fs::create_dir_all(&self.stage_dir);
        let Ok(files) = std::fs::read_dir(&self.stage_dir) else { return };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            let staged = name.strip_prefix("Telinha-").and_then(|rest| rest.strip_suffix(".exe")).and_then(parse_version);
            let stale = matches!((staged, current), (Some(staged), Some(current)) if staged <= current);
            if name.ends_with(".part") || name == HELPER_NAME || stale {
                // What cannot be removed now waits for the next time the app opens.
                let _ = std::fs::remove_file(file.path());
            }
        }
    }

    fn set(&self, change: impl FnOnce(&mut UpdateState)) {
        let state = {
            let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            change(&mut state);
            state.clone()
        };
        (self.listener)(UpdateEvent::Changed(state));
    }

    /// The redirect of the page of the latest release tells its tag without spending the hourly
    /// limit of the API, so it can be asked every minute.
    async fn latest_version(&self) -> Result<Option<[u64; 3]>, String> {
        let response = self.page.head(&self.endpoints.release_page).timeout(REQUEST_TIMEOUT).send().await.map_err(|error| error.to_string())?;
        let location = response.headers().get(reqwest::header::LOCATION).and_then(|value| value.to_str().ok()).unwrap_or("");
        Ok(location.rsplit_once("/releases/tag/").and_then(|(_, tag)| parse_version(tag)))
    }

    async fn describe(&self) -> Result<Option<Update>, String> {
        let response = self
            .client
            .get(&self.endpoints.release_api)
            .timeout(REQUEST_TIMEOUT)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("o GitHub respondeu {}", response.status().as_u16()));
        }
        let body = response.bytes().await.map_err(|error| error.to_string())?;
        let release: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
        Ok(pick_update(&release, &self.current, &self.endpoints.download_prefix))
    }

    /// Looks for a newer release and downloads it. Does nothing while a download is ready.
    pub async fn check(&self) {
        let waiting = self.retry_at.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_some_and(|at| Instant::now() < at);
        if waiting || self.applying.load(Ordering::SeqCst) || self.snapshot().status == UpdateStatus::Ready || self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Err(error) = self.look().await {
            *self.retry_at.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now() + RETRY_AFTER_FAILURE);
            self.set(|state| *state = UpdateState { status: UpdateStatus::Idle, version: None, progress: 0.0 });
            (self.listener)(UpdateEvent::Failed(error));
        }
        self.busy.store(false, Ordering::SeqCst);
    }

    async fn look(&self) -> Result<(), String> {
        let (Some(latest), Some(current)) = (self.latest_version().await?, parse_version(&self.current)) else { return Ok(()) };
        if latest <= current {
            return Ok(());
        }
        let version = written(latest);
        // A release published without raising the version would bring a program that still says
        // the old one, and it would be downloaded and swapped again without end.
        if self.read_mark(APPLIED_FILE) == version {
            return Ok(());
        }
        if !self.staged_file(&version).is_file() {
            self.fetch_release(&version).await?;
        }
        let status = if self.read_mark(FAILED_FILE) == version { UpdateStatus::Blocked } else { UpdateStatus::Ready };
        self.set(|state| *state = UpdateState { status, version: Some(version), progress: 1.0 });
        Ok(())
    }

    async fn fetch_release(&self, version: &str) -> Result<(), String> {
        let update = self.describe().await?.filter(|update| update.version == version).ok_or("a release mudou no meio da conferida")?;
        let staged = self.staged_file(version);
        let partial = staged.with_extension("exe.part");
        self.set(|state| *state = UpdateState { status: UpdateStatus::Downloading, version: Some(version.to_owned()), progress: 0.0 });
        let downloaded = self.download(&update, &partial).await;
        if downloaded.is_err() {
            let _ = tokio::fs::remove_file(&partial).await;
        }
        downloaded?;
        tokio::fs::rename(&partial, &staged).await.map_err(|error| error.to_string())
    }

    async fn download(&self, update: &Update, destination: &Path) -> Result<(), String> {
        let mut response = self
            .client
            .get(&update.url)
            .header(reqwest::header::ACCEPT, "application/octet-stream")
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("o download respondeu {}", response.status().as_u16()));
        }
        let _ = tokio::fs::create_dir_all(&self.stage_dir).await;
        let mut file = tokio::fs::File::create(destination).await.map_err(|error| error.to_string())?;
        let mut hash = Sha256::new();
        let mut received = 0u64;
        let mut shown = u64::MAX;
        loop {
            let chunk = tokio::time::timeout(STALL_TIMEOUT, response.chunk()).await.map_err(|_| "o download parou no meio")?.map_err(|error| error.to_string())?;
            let Some(chunk) = chunk else { break };
            received += chunk.len() as u64;
            if received > update.size {
                return Err("o download veio maior que o anunciado".into());
            }
            hash.update(&chunk);
            file.write_all(&chunk).await.map_err(|error| error.to_string())?;
            let percent = received * 100 / update.size;
            if percent != shown {
                shown = percent;
                self.set(|state| state.progress = received as f32 / update.size as f32);
            }
        }
        file.flush().await.map_err(|error| error.to_string())?;
        drop(file);
        if received != update.size {
            return Err("o download veio incompleto".into());
        }
        let digest: String = hash.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
        if update.sha256.as_ref().is_some_and(|expected| *expected != digest) {
            return Err("o arquivo baixado não confere com o da release".into());
        }
        Ok(())
    }

    /// Hands the swap to the engine, which does it once the process `wait_pid` ends. So whoever
    /// calls this closes the app right after. `relaunch` opens the app again after the swap.
    /// `elevate` lets it ask for the permission of Windows when the folder of the program is
    /// protected, which only makes sense with the person in front of the computer.
    pub fn apply(&self, wait_pid: u32, relaunch: bool, elevate: bool) -> bool {
        let state = self.snapshot();
        // After a refused permission, only a try with the person present is worth it.
        let allowed = state.status == UpdateStatus::Ready || (state.status == UpdateStatus::Blocked && elevate);
        let Some(version) = state.version.filter(|_| allowed && !self.applying.load(Ordering::SeqCst)) else { return false };
        if elevate {
            // A new swap writes over the mark if it fails again.
            let _ = std::fs::remove_file(self.stage_dir.join(FAILED_FILE));
        }
        let request = ApplyRequest {
            helper: self.helper.clone(),
            source: self.staged_file(&version),
            target: self.target.clone(),
            version,
            wait_pid,
            applied_file: self.stage_dir.join(APPLIED_FILE),
            failed_file: self.stage_dir.join(FAILED_FILE),
            relaunch,
            elevate,
        };
        match (self.apply)(&request) {
            Ok(()) => {
                self.applying.store(true, Ordering::SeqCst);
                true
            }
            Err(error) => {
                // Without the helper the app stays open on the version it has.
                (self.listener)(UpdateEvent::Failed(error));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[test]
    fn a_version_is_three_numbers_and_nothing_else() {
        assert_eq!(parse_version("v0.3.8"), Some([0, 3, 8]));
        assert_eq!(parse_version(" 12.0.1\n"), Some([12, 0, 1]));
        for odd in ["", "1.2", "1.2.3.4", "1.2.x", "v1.2.3-beta", "1..3", "-1.2.3"] {
            assert_eq!(parse_version(odd), None, "{odd}");
        }
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn only_a_newer_finished_release_with_the_right_file_is_taken() {
        let prefix = "https://github.com/Raphalsk050/Telinha/releases/download/";
        let digest = "ab".repeat(32);
        let release = |change: &dyn Fn(&mut Value)| {
            let mut release = json!({
                "tag_name": "v0.4.0", "draft": false, "prerelease": false,
                "assets": [
                    { "name": "notes.txt", "browser_download_url": format!("{prefix}v0.4.0/notes.txt"), "size": 10 },
                    { "name": "Telinha.exe", "browser_download_url": format!("{prefix}v0.4.0/Telinha.exe"), "size": 1000, "digest": format!("sha256:{digest}") },
                ],
            });
            change(&mut release);
            release
        };
        let picked = pick_update(&release(&|_| {}), "0.3.8", prefix).unwrap();
        assert_eq!(picked, Update { version: "0.4.0".into(), url: format!("{prefix}v0.4.0/Telinha.exe"), size: 1000, sha256: Some(digest.clone()) });
        assert_eq!(pick_update(&release(&|_| {}), "0.4.0", prefix), None);
        assert_eq!(pick_update(&release(&|release| release["draft"] = json!(true)), "0.3.8", prefix), None);
        assert_eq!(pick_update(&release(&|release| release["prerelease"] = json!(true)), "0.3.8", prefix), None);
        assert_eq!(pick_update(&release(&|release| release["assets"][1]["browser_download_url"] = json!("https://evil.example/Telinha.exe")), "0.3.8", prefix), None);
        assert_eq!(pick_update(&release(&|release| release["assets"][1]["size"] = json!(0)), "0.3.8", prefix), None);
        assert_eq!(pick_update(&release(&|release| release["assets"][1]["name"] = json!("Other.exe")), "0.3.8", prefix), None);
        let unsigned = pick_update(&release(&|release| release["assets"][1]["digest"] = json!("md5:abc")), "0.3.8", prefix).unwrap();
        assert_eq!(unsigned.sha256, None);
    }

    /// A release server on this machine: the page that redirects to the tag, the description of
    /// the release and the file.
    async fn serve(tag: &'static str, file: Vec<u8>, digest: String) -> (Endpoints, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let endpoints = Endpoints { release_page: format!("{base}/releases/latest"), release_api: format!("{base}/api/latest"), download_prefix: format!("{base}/download/") };
        let description = json!({
            "tag_name": tag, "draft": false, "prerelease": false,
            "assets": [{ "name": "Telinha.exe", "browser_download_url": format!("{base}/download/{tag}/Telinha.exe"), "size": file.len(), "digest": format!("sha256:{digest}") }],
        })
        .to_string();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let (description, file, base) = (description.clone(), file.clone(), base.clone());
                tokio::spawn(async move {
                    let mut request = vec![0u8; 4096];
                    let read = socket.read(&mut request).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..read]).into_owned();
                    let line = request.lines().next().unwrap_or("");
                    let reply: Vec<u8> = if line.starts_with("HEAD /releases/latest") {
                        format!("HTTP/1.1 302 Found\r\nLocation: {base}/releases/tag/{tag}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
                    } else if line.starts_with("GET /api/latest") {
                        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{description}", description.len()).into_bytes()
                    } else if line.starts_with("GET /download/") {
                        let mut reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", file.len()).into_bytes();
                        reply.extend_from_slice(&file);
                        reply
                    } else {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                    };
                    let _ = socket.write_all(&reply).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (endpoints, server)
    }

    fn digest_of(data: &[u8]) -> String {
        Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[tokio::test]
    async fn a_newer_release_is_downloaded_checked_and_handed_to_the_swap() {
        let file: Vec<u8> = (0..300_000u32).map(|index| (index % 251) as u8).collect();
        let (endpoints, server) = serve("v9.9.9", file.clone(), digest_of(&file)).await;
        let folder = tempfile::tempdir().unwrap();
        let stage = folder.path().join("updates");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(stage.join("Telinha-0.1.0.exe"), b"old").unwrap();
        std::fs::write(stage.join("Telinha-9.9.8.exe.part"), b"half").unwrap();

        let heard = Arc::new(Mutex::new(Vec::new()));
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (listener, requests) = (Arc::clone(&heard), Arc::clone(&asked));
        let updater = Updater::new("0.3.8", &folder.path().join("Telinha.exe"), &stage, &folder.path().join("engine.exe"), endpoints, move |event| listener.lock().unwrap().push(event))
            .with_apply(move |request| {
                requests.lock().unwrap().push(request.clone());
                Ok(())
            });
        updater.remove_leftovers();
        assert!(!stage.join("Telinha-0.1.0.exe").exists() && !stage.join("Telinha-9.9.8.exe.part").exists());
        assert!(!updater.apply(1, true, true), "nothing to apply before a download");

        updater.check().await;
        assert_eq!(updater.snapshot(), UpdateState { status: UpdateStatus::Ready, version: Some("9.9.9".into()), progress: 1.0 });
        assert_eq!(std::fs::read(stage.join("Telinha-9.9.9.exe")).unwrap(), file);
        let events = heard.lock().unwrap().clone();
        assert!(matches!(&events[0], UpdateEvent::Changed(state) if state.status == UpdateStatus::Downloading && state.progress == 0.0));
        assert!(!events.iter().any(|event| matches!(event, UpdateEvent::Failed(_))));

        assert!(updater.apply(4242, false, false));
        assert!(!updater.apply(4242, false, false), "the swap is handed over once");
        let request = asked.lock().unwrap()[0].clone();
        assert_eq!(request.source, stage.join("Telinha-9.9.9.exe"));
        assert_eq!((request.version.as_str(), request.wait_pid, request.relaunch, request.elevate), ("9.9.9", 4242, false, false));
        assert_eq!(request.applied_file, stage.join("applied.txt"));
        server.abort();
    }

    #[tokio::test]
    async fn a_file_that_does_not_match_the_release_is_thrown_away_and_a_swapped_version_is_left_alone() {
        let file = b"not what the release says".to_vec();
        let (endpoints, server) = serve("v9.9.9", file, "0".repeat(64)).await;
        let folder = tempfile::tempdir().unwrap();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let listener = Arc::clone(&heard);
        let updater = Updater::new("0.3.8", &folder.path().join("Telinha.exe"), folder.path(), &folder.path().join("engine.exe"), endpoints.clone(), move |event| listener.lock().unwrap().push(event));
        updater.check().await;
        assert_eq!(updater.snapshot().status, UpdateStatus::Idle);
        assert!(heard.lock().unwrap().contains(&UpdateEvent::Failed("o arquivo baixado não confere com o da release".into())));
        assert!(std::fs::read_dir(folder.path()).unwrap().flatten().all(|file| !file.file_name().to_string_lossy().starts_with("Telinha-")));

        // The same version already swapped once is not downloaded again.
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("applied.txt"), "9.9.9\n").unwrap();
        let quiet = Updater::new("0.3.8", &other.path().join("Telinha.exe"), other.path(), &other.path().join("engine.exe"), endpoints, |_| {});
        quiet.check().await;
        assert_eq!(quiet.snapshot().status, UpdateStatus::Idle);
        assert!(!other.path().join("Telinha-9.9.9.exe").exists());
        server.abort();
    }
}
