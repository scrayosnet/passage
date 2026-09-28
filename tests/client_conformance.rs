//! The official Minecraft client, driven against the real Passage binary, once per protocol
//! breakpoint.
//!
//! Every other test in this repository proves Passage agrees with itself. The flow tests replay
//! the whole conversation at every supported version, which is worth having -- but both ends of
//! them share [`Packet::IDS`](passage_core::Packet::IDS) and the same encoder, so a packet ID we
//! got wrong, or a field gated at the wrong version, makes the two halves agree with each other
//! and pass. Only Minecraft can tell us we read the protocol documentation wrong.
//!
//! So this test asks Minecraft. It installs the official client the way a launcher does, points it
//! at a Passage that was started from the shipped binary, and watches what the client says. A
//! transfer leaves an unforgeable trace in the client's own log:
//!
//! ```text
//! Connecting to 127.0.0.1, <passage port>
//! Client disconnected with reason: Transferred to another server
//! Connecting to 127.0.0.1, <backend port>
//! ```
//!
//! The third line is the point: the client parsed our transfer packet and acted on it. A landing
//! socket then confirms the same thing from the other side, by reading the handshake the client
//! sends it -- parsed by hand here rather than with `passage-core`, so that the assertion does not
//! lean on the code under test.
//!
//! # Running it
//!
//! ```text
//! xvfb-run -a cargo test -p passage --test client_conformance -- --ignored --nocapture
//! ```
//!
//! `xvfb-run` only supplies a display. On a desktop session there is already one, so it can be
//! dropped -- at the cost of seven Minecraft windows opening in turn and taking the focus with
//! them. Two variables make that bearable:
//!
//! ```text
//! # one version instead of the whole matrix
//! PASSAGE_CLIENT_VERSIONS=26.3 cargo test -p passage --test client_conformance -- --ignored --nocapture
//! # and the real GPU instead of llvmpipe, which is only the right default for a headless runner
//! LIBGL_ALWAYS_SOFTWARE=0 GALLIUM_DRIVER= MESA_LOADER_DRIVER_OVERRIDE= PASSAGE_CLIENT_VERSIONS=26.3 \
//!   cargo test -p passage --test client_conformance -- --ignored --nocapture
//! ```
//!
//! It needs an X display, a JDK of at least 25, Mesa for software rendering, and several gigabytes
//! of cache for client jars, libraries and assets. The first run downloads all of it; later runs
//! reuse it. Assets are content-addressed, so the seven versions share most of theirs.
//!
//! Everything the client writes -- logs, crash reports, the LWJGL native cache -- goes under the
//! run directory rather than the working directory, so running this from the repository root
//! leaves nothing behind in it.
//!
//! 26.3 replaced GLFW with SDL3 and prefers Vulkan: without a software Vulkan driver
//! (`mesa-vulkan-drivers`, which provides lavapipe) that version does not fail, it *hangs*, which
//! is why every launch here is bounded by [`DEADLINE`] rather than trusted to exit.
//!
//! Nothing runs this in CI. It is `#[ignore]`d, and it is a nightly job at most: a client boot
//! costs the better part of a minute and a couple of gigabytes of RAM.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

/// Mojang's index of every released version and where its metadata lives.
const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// One Minecraft version per protocol breakpoint: the versions where something in the four phases
/// Passage speaks actually changed. The releases in between are served by the same tables, which
/// `passage-core`'s conformance test proves, so booting a client for them would buy nothing.
const BREAKPOINTS: &[(&str, i32)] = &[
    ("1.20.6", 766),
    ("1.21", 767),
    ("1.21.2", 768),
    ("1.21.6", 771),
    ("1.21.9", 773),
    ("26.2", 776),
    ("26.3", 777),
];

/// How long one client gets to reach the transfer before it is killed. Generous, because a cold
/// llvmpipe boot is slow and 26.3 without a Vulkan driver hangs rather than failing.
const DEADLINE: Duration = Duration::from_secs(300);

/// The hostname the route matches and the client dials.
const HOST: &str = "127.0.0.1";

/// Prints progress past libtest's capture, which would otherwise swallow it for minutes.
macro_rules! progress {
    ($($arg:tt)*) => {{
        let mut err = std::io::stderr();
        let _ = writeln!(err, $($arg)*);
        let _ = err.flush();
    }};
}

/// Where client jars, libraries and assets are kept between runs.
///
/// `PASSAGE_CLIENT_CACHE` moves it, which is what lets CI put it somewhere it can cache between
/// nightly runs -- the first run downloads several gigabytes and every run after it downloads
/// nothing.
fn cache() -> PathBuf {
    std::env::var_os("PASSAGE_CLIENT_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("passage-client-conformance"))
}

/// A child process that is killed when the test leaves its scope, however it leaves.
///
/// Load-bearing on a failure: a panic in the middle of a launch would otherwise leave a Minecraft
/// client and a Passage running for as long as the test binary does.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Downloads `url` into `path`, or leaves the cached copy alone.
async fn fetch(client: &reqwest::Client, url: &str, path: &Path) -> Vec<u8> {
    if let Ok(cached) = std::fs::read(path) {
        return cached;
    }
    let body = client
        .get(url)
        .send()
        .await
        .unwrap_or_else(|err| panic!("GET {url}: {err}"))
        .error_for_status()
        .unwrap_or_else(|err| panic!("GET {url}: {err}"))
        .bytes()
        .await
        .unwrap_or_else(|err| panic!("GET {url}: {err}"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the cache directory is writable");
    }
    std::fs::write(path, &body).expect("the cache is writable");
    body.to_vec()
}

/// Whether a library's rules allow it on 64-bit Linux.
///
/// A library carries a list of allow/disallow rules, last match winning, and one with no rules at
/// all is unconditional. Rules keyed on `features` belong to the launcher's optional arguments
/// rather than to a library, so anything that mentions them is skipped.
fn allowed_here(rules: Option<&Vec<serde_json::Value>>) -> bool {
    let Some(rules) = rules else {
        return true;
    };
    let mut allowed = false;
    for rule in rules {
        if rule.get("features").is_some() {
            continue;
        }
        let os = &rule["os"];
        let name_matches = os["name"].as_str().is_none_or(|name| name == "linux");
        let arch_matches = os["arch"].as_str().is_none_or(|arch| arch == "x86_64");
        if name_matches && arch_matches {
            allowed = rule["action"] == "allow";
        }
    }
    allowed
}

/// An installed client: everything needed to build a command line.
struct Install {
    classpath: Vec<PathBuf>,
    main_class: String,
    asset_index: String,
    assets: PathBuf,
}

/// Installs one Minecraft version the way a launcher does: the client jar, every library its rules
/// allow, and the assets its index names.
///
/// Native libraries are put on the classpath rather than extracted. LWJGL unpacks its own natives
/// from there when `java.library.path` names none, which saves this test an archive decoder.
async fn install(client: &reqwest::Client, manifest: &serde_json::Value, version: &str) -> Install {
    let url = manifest["versions"]
        .as_array()
        .expect("the manifest lists versions")
        .iter()
        .find(|entry| entry["id"] == version)
        .unwrap_or_else(|| panic!("{version} is not in the version manifest"))["url"]
        .as_str()
        .expect("a url")
        .to_owned();
    let meta: serde_json::Value = serde_json::from_slice(
        &fetch(client, &url, &cache().join(format!("meta/{version}.json"))).await,
    )
    .expect("the metadata is JSON");

    let jar = cache().join(format!("versions/{version}/client.jar"));
    fetch(
        client,
        meta["downloads"]["client"]["url"].as_str().expect("a jar"),
        &jar,
    )
    .await;
    // The client jar leads the classpath, and every library its rules allow follows.
    let mut classpath = vec![jar];

    for library in meta["libraries"].as_array().expect("libraries") {
        if !allowed_here(library["rules"].as_array()) {
            continue;
        }
        let mut artifacts = vec![&library["downloads"]["artifact"]];
        // Older versions keep their natives in a `classifiers` map rather than as libraries of
        // their own, and only the Linux one is any use here.
        if let Some(classifiers) = library["downloads"]["classifiers"].as_object() {
            artifacts.extend(
                classifiers
                    .iter()
                    .filter(|(key, _)| key.contains("linux"))
                    .map(|(_, value)| value),
            );
        }
        for artifact in artifacts {
            let (Some(path), Some(url)) = (artifact["path"].as_str(), artifact["url"].as_str())
            else {
                continue;
            };
            let local = cache().join("libraries").join(path);
            fetch(client, url, &local).await;
            classpath.push(local);
        }
    }

    let index_id = meta["assetIndex"]["id"].as_str().expect("an asset index");
    let assets = cache().join("assets");
    let index = fetch(
        client,
        meta["assetIndex"]["url"].as_str().expect("a url"),
        &assets.join(format!("indexes/{index_id}.json")),
    )
    .await;

    // The objects are content-addressed, so the versions share most of theirs and a second version
    // costs a fraction of the first.
    let index: serde_json::Value = serde_json::from_slice(&index).expect("the index is JSON");
    let objects = index["objects"].as_object().expect("objects");
    let mut pending = Vec::new();
    for object in objects.values() {
        let hash = object["hash"].as_str().expect("a hash").to_owned();
        let local = assets.join(format!("objects/{}/{hash}", &hash[..2]));
        if !local.exists() {
            pending.push((hash, local));
        }
    }
    if !pending.is_empty() {
        progress!("[{version}] fetching {} assets", pending.len());
        let mut running = tokio::task::JoinSet::new();
        for (hash, local) in pending {
            let client = client.clone();
            // Bounded so the test is a good citizen against Mojang's CDN.
            while running.len() >= 16 {
                running.join_next().await;
            }
            running.spawn(async move {
                let url = format!(
                    "https://resources.download.minecraft.net/{}/{hash}",
                    &hash[..2]
                );
                fetch(&client, &url, &local).await;
            });
        }
        while running.join_next().await.is_some() {}
    }

    Install {
        classpath,
        main_class: meta["mainClass"]
            .as_str()
            .unwrap_or("net.minecraft.client.main.Main")
            .to_owned(),
        asset_index: index_id.to_owned(),
        assets,
    }
}

/// Picks a port nothing is listening on, by binding one and letting go.
fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("a free port")
        .local_addr()
        .expect("an address")
        .port()
}

/// Starts the Passage binary on `port`, routing everything to `target`, and waits for it to listen.
///
/// The shipped binary rather than a harness, so what is proven includes the configuration it reads
/// and the listener it stands up.
fn serve_passage(port: u16, target: SocketAddr, dir: &Path) -> Reaped {
    let config = dir.join("passage.yaml");
    std::fs::write(
        &config,
        format!(
            "address: '{HOST}:{port}'\n\
             routes:\n\
             - hostname: '.*'\n\
            \x20 status:\n\
            \x20   type: 'fixed'\n\
            \x20   name: 'Passage conformance'\n\
            \x20 authentication:\n\
            \x20   type: 'disabled'\n\
            \x20 localization:\n\
            \x20   type: 'fixed'\n\
            \x20   default_locale: 'en_US'\n\
            \x20 discovery:\n\
            \x20   type: 'fixeddiscovery'\n\
            \x20   targets:\n\
            \x20   - identifier: 'backend'\n\
            \x20     address: '{target}'\n",
        ),
    )
    .expect("the config is writable");

    let passage = Command::new(env!("CARGO_BIN_EXE_passage"))
        .env("CONFIG_FILE", &config)
        .env("RUST_LOG", "info")
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the passage binary runs");
    let passage = Reaped(passage);

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if TcpStream::connect((HOST, port)).is_ok() {
            return passage;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("passage did not start listening on {HOST}:{port}");
}

/// What the landing socket saw: the handshake a transferred client opens with.
#[derive(Debug)]
struct Landing {
    protocol: i32,
    host: String,
    port: u16,
    intent: i32,
}

/// Reads the handshake a client sends, by hand.
///
/// Deliberately not `passage-core`'s decoder: this is the half of the test that speaks for
/// Minecraft, so it must not share code with what it is judging.
fn read_handshake(stream: &mut TcpStream) -> Landing {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .expect("a read timeout");
    let mut reader = BufReader::new(stream);
    let byte = |reader: &mut BufReader<&mut TcpStream>| -> u8 {
        let mut buf = [0u8; 1];
        std::io::Read::read_exact(reader, &mut buf).expect("the client sends a handshake");
        buf[0]
    };
    let var_int = |reader: &mut BufReader<&mut TcpStream>| -> i32 {
        let (mut value, mut shift) = (0i32, 0);
        loop {
            let read = byte(reader);
            value |= i32::from(read & 0x7F) << shift;
            if read & 0x80 == 0 {
                return value;
            }
            shift += 7;
            assert!(shift < 35, "a varint that never ends");
        }
    };

    let _length = var_int(&mut reader);
    let id = var_int(&mut reader);
    assert_eq!(
        id, 0x00,
        "the first packet of a connection is the handshake"
    );
    let protocol = var_int(&mut reader);
    let host_len = var_int(&mut reader);
    let mut host = vec![0u8; usize::try_from(host_len).expect("a sane hostname length")];
    std::io::Read::read_exact(&mut reader, &mut host).expect("a hostname");
    let port = u16::from_be_bytes([byte(&mut reader), byte(&mut reader)]);
    let intent = var_int(&mut reader);
    Landing {
        protocol,
        host: String::from_utf8(host).expect("a UTF-8 hostname"),
        port,
        intent,
    }
}

/// Builds the command line a launcher would, and starts the client with its output on a channel.
fn launch(
    install: &Install,
    version: &str,
    dir: &Path,
    target: &str,
) -> (Reaped, Receiver<String>) {
    let game = dir.join("game");
    std::fs::create_dir_all(&game).expect("the game directory is writable");
    // A fresh game directory parks on the accessibility onboarding screen and never connects, so
    // the options it would have written are written first.
    std::fs::write(
        game.join("options.txt"),
        "onboardAccessibility:false\nskipMultiplayerWarning:true\ntutorialStep:none\n\
         pauseOnLostFocus:false\nsoundCategory_master:0.0\nrenderDistance:2\nmaxFps:30\n",
    )
    .expect("the options are writable");
    let tmp = dir.join("tmp");
    std::fs::create_dir_all(&tmp).expect("the temp directory is writable");

    let classpath = install
        .classpath
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":");

    let mut command = Command::new("java");
    command
        .arg(format!("-Djava.io.tmpdir={}", tmp.display()))
        .arg(format!("-Djna.tmpdir={}", tmp.display()))
        .args(["-Xmx2G", "--enable-native-access=ALL-UNNAMED", "-cp"])
        .arg(&classpath)
        .arg(&install.main_class)
        .args(["--username", "Probe"])
        .args(["--version", version])
        .arg("--gameDir")
        .arg(&game)
        .arg("--assetsDir")
        .arg(&install.assets)
        .args(["--assetIndex", &install.asset_index])
        // An offline session: the access token is never checked, because the route Passage serves
        // authenticates nobody and therefore does not ask the client to prove anything.
        .args(["--uuid", "3f0ca0c89f163b8792fbb626b9a1bb99"])
        .args(["--accessToken", "0"])
        .args(["--clientId", "", "--xuid", ""])
        // Removed in 25w31a; the client ignores arguments it does not know, so it is safe on both
        // sides of that line.
        .args(["--userType", "legacy"])
        .args(["--versionType", "release"])
        .args(["--quickPlayMultiplayer", target])
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Software rendering under Xvfb, and the EGL path 26.3's SDL3 backend needs; older versions
    // ignore the SDL variable entirely. Each is a default rather than an override, so a run on a
    // desktop with a real GPU keeps its own driver simply by exporting the variable first --
    // llvmpipe is what a headless runner needs, not what a developer watching the window wants.
    for (key, value) in [
        ("LIBGL_ALWAYS_SOFTWARE", "1"),
        ("GALLIUM_DRIVER", "llvmpipe"),
        ("MESA_LOADER_DRIVER_OVERRIDE", "llvmpipe"),
        ("SDL_VIDEO_FORCE_EGL", "1"),
    ] {
        if std::env::var_os(key).is_none() {
            command.env(key, value);
        }
    }

    let mut child = command.spawn().expect("java is on the PATH");
    let (sender, receiver) = channel();
    for stream in [
        Box::new(child.stdout.take().expect("stdout is piped")) as Box<dyn std::io::Read + Send>,
        Box::new(child.stderr.take().expect("stderr is piped")),
    ] {
        let sender = sender.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
    }
    (Reaped(child), receiver)
}

/// Drives one version end to end and returns what went wrong, if anything.
async fn transfer_once(
    client: &reqwest::Client,
    manifest: &serde_json::Value,
    version: &str,
    protocol: i32,
) -> Result<(), String> {
    progress!("[{version}] installing");
    let install = install(client, manifest, version).await;

    let dir = cache().join(format!("run/{version}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the run directory is writable");

    // The landing socket the transfer points at. It is never a Minecraft server: the client only
    // has to arrive, and what it says on arrival is the proof.
    let landing = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a landing socket");
    let landing_addr = SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::LOCALHOST,
        landing.local_addr().expect("an address").port(),
    ));

    let port = free_port();
    let _passage = serve_passage(port, landing_addr, &dir);
    progress!("[{version}] passage on {HOST}:{port}, landing on {landing_addr}");

    // The landing socket reports what arrived over a channel rather than being joined, so the
    // loop below can watch for it and for the client's output at the same time.
    let (landed_tx, landed_rx) = channel();
    std::thread::spawn(move || {
        if let Some(Ok(mut stream)) = landing.incoming().next() {
            let _ = landed_tx.send(read_handshake(&mut stream));
        }
    });

    let (_client, lines) = launch(&install, version, &dir, &format!("{HOST}:{port}"));
    progress!("[{version}] client launched, waiting for the transfer");

    // What the client says is context for a failure; what arrives at the landing socket is the
    // verdict. Only the first line is required -- the middle one reads
    // "Client disconnected with reason: Transferred to another server" only once the language
    // files have loaded, and falls back to the bare `disconnect.transfer` key otherwise.
    let dialled = format!("Connecting to {HOST}, {port}");
    let mut saw_dialled = false;

    let deadline = Instant::now() + DEADLINE;
    let mut tail = Vec::new();
    let landing = loop {
        match landed_rx.try_recv() {
            Ok(landing) => break landing,
            Err(TryRecvError::Disconnected) => {
                return Err(format!(
                    "[{version}] the landing socket closed without a handshake; last lines:\n{}",
                    tail.join("\n"),
                ));
            }
            Err(TryRecvError::Empty) => {}
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "[{version}] timed out after {}s without a transfer (the client {} dial \
                 Passage); last lines:\n{}",
                DEADLINE.as_secs(),
                if saw_dialled { "did" } else { "never did" },
                tail.join("\n"),
            ));
        }
        match lines.try_recv() {
            Ok(line) => {
                if line.contains(&dialled) {
                    saw_dialled = true;
                }
                if tail.len() == 40 {
                    tail.remove(0);
                }
                tail.push(line);
            }
            Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(50)),
            Err(TryRecvError::Disconnected) => {
                // The client died. Give the landing socket a moment in case it arrived first.
                std::thread::sleep(Duration::from_millis(200));
                if let Ok(landing) = landed_rx.try_recv() {
                    break landing;
                }
                return Err(format!(
                    "[{version}] the client exited before transferring (it {} dial Passage); \
                     last lines:\n{}",
                    if saw_dialled { "did" } else { "never did" },
                    tail.join("\n"),
                ));
            }
        }
    };

    if !saw_dialled {
        return Err(format!(
            "[{version}] something reached the landing socket, but the client never logged \
             dialling Passage -- the transfer did not come from this run",
        ));
    }
    if landing.intent != 3 {
        return Err(format!(
            "[{version}] the client arrived with intent {} rather than 3 (transfer)",
            landing.intent,
        ));
    }
    if landing.protocol != protocol {
        return Err(format!(
            "[{version}] the client announced protocol {} rather than {protocol}",
            landing.protocol,
        ));
    }
    progress!(
        "[{version}] transferred: protocol {}, intent {}, to {}:{}",
        landing.protocol,
        landing.intent,
        landing.host,
        landing.port,
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "boots the official Minecraft client; needs an X display, a JDK and gigabytes of cache"]
async fn the_official_client_follows_a_transfer_at_every_breakpoint() {
    assert!(
        std::env::var_os("DISPLAY").is_some(),
        "no DISPLAY -- run this under `xvfb-run -a cargo test ...`, or on a desktop session, \
         where the client opens a real window instead",
    );

    // `PASSAGE_CLIENT_VERSIONS=26.3,26.2` narrows the matrix, which is what makes a local run
    // bearable: the whole list is seven clients, one after another, each allowed several minutes.
    let filter = std::env::var("PASSAGE_CLIENT_VERSIONS").ok();
    let wanted: Vec<&(&str, i32)> = BREAKPOINTS
        .iter()
        .filter(|(version, _)| {
            filter
                .as_deref()
                .is_none_or(|filter| filter.split(',').any(|want| want.trim() == *version))
        })
        .collect();
    assert!(
        !wanted.is_empty(),
        "PASSAGE_CLIENT_VERSIONS matched nothing; the breakpoints are {:?}",
        BREAKPOINTS.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
    );

    std::fs::create_dir_all(cache()).expect("the cache directory is writable");
    progress!("caching clients in {}", cache().display());

    let http = reqwest::Client::builder()
        .user_agent(concat!(
            "passage-client-conformance/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .expect("an HTTP client");
    let manifest: serde_json::Value = serde_json::from_slice(
        &fetch(&http, MANIFEST, &cache().join("version_manifest_v2.json")).await,
    )
    .expect("the manifest is JSON");

    // Every version is attempted even after one fails, because "26.3 broke" and "everything broke"
    // call for very different mornings.
    let mut failures = Vec::new();
    for (version, protocol) in wanted.iter().copied() {
        if let Err(failure) = transfer_once(&http, &manifest, version, *protocol).await {
            progress!("{failure}");
            failures.push(failure);
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} clients did not follow the transfer:\n\n{}",
        failures.len(),
        wanted.len(),
        failures.join("\n\n"),
    );
}

/// Sends `bytes` as a length-prefixed frame.
fn send_frame(stream: &mut TcpStream, payload: &[u8]) {
    let mut length = Vec::new();
    let mut remaining = u32::try_from(payload.len()).expect("a small frame");
    loop {
        let byte = u8::try_from(remaining & 0x7F).expect("seven bits");
        remaining >>= 7;
        if remaining == 0 {
            length.push(byte);
            break;
        }
        length.push(byte | 0x80);
    }
    std::io::Write::write_all(stream, &length).expect("writes");
    std::io::Write::write_all(stream, payload).expect("writes");
}

/// A handshake packet, built by hand.
fn handshake_bytes(protocol: i32, host: &str, port: u16, intent: u8) -> Vec<u8> {
    let mut out = vec![0x00];
    let mut remaining = u32::try_from(protocol).expect("a positive protocol");
    loop {
        let byte = u8::try_from(remaining & 0x7F).expect("seven bits");
        remaining >>= 7;
        if remaining == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
    out.push(u8::try_from(host.len()).expect("a short hostname"));
    out.extend_from_slice(host.as_bytes());
    out.extend_from_slice(&port.to_be_bytes());
    out.push(intent);
    out
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "starts the passage binary; needs no display, no JDK and no downloads"]
async fn the_harness_starts_passage_and_reads_a_handshake() {
    // The half of the harness that can be checked without a Minecraft client: the configuration
    // the binary is handed, the port it listens on, and the hand-rolled handshake reader the
    // landing socket judges a transfer with.
    let dir = cache().join("preflight");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the run directory is writable");

    let landing = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a landing socket");
    let landing_addr = SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::LOCALHOST,
        landing.local_addr().expect("an address").port(),
    ));
    let port = free_port();
    let _passage = serve_passage(port, landing_addr, &dir);

    // A status ping, written by hand, answered by the shipped binary.
    let mut peer = TcpStream::connect((HOST, port)).expect("passage accepts");
    send_frame(&mut peer, &handshake_bytes(777, HOST, port, 1));
    send_frame(&mut peer, &[0x00]);
    peer.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a read timeout");
    let mut head = [0u8; 3];
    std::io::Read::read_exact(&mut peer, &mut head).expect("a status response arrives");
    assert_eq!(head[head.len() - 1], 0x00, "the status response leads");

    // And the landing reader, against a handshake it did not write.
    let arrived = std::thread::spawn(move || {
        read_handshake(
            &mut landing
                .incoming()
                .next()
                .expect("a connection")
                .expect("accepts"),
        )
    });
    let mut client = TcpStream::connect(landing_addr).expect("the landing socket accepts");
    send_frame(
        &mut client,
        &handshake_bytes(777, "mc.justchunks.net", 25_565, 3),
    );
    let landed = arrived.join().expect("the reader does not panic");
    assert_eq!(landed.protocol, 777);
    assert_eq!(landed.host, "mc.justchunks.net");
    assert_eq!(landed.port, 25_565);
    assert_eq!(landed.intent, 3, "a transfer");
}
