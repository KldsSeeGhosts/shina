//! Fresh executor clones over loopback Git and authenticated peer transport.
//! No network host, provider account, credential helper or real profile is used.
use super::*;

fn accepted(reply: Value) -> Value {
    assert_eq!(reply["result"]["status"], "accepted", "{reply}");
    reply["result"]["result"].clone()
}

fn hello(client: &mut Client) {
    let result = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!(uuid::Uuid::new_v4()),
    );
    assert!(result["error"].is_null(), "{result}");
}

fn git(path: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

struct GitDaemon(Child);
impl Drop for GitDaemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl GitDaemon {
    fn start(root: &std::path::Path) -> (Self, String, String) {
        let served = root.join("served");
        let repository = served.join("repo");
        fs::create_dir_all(&repository).unwrap();
        git(&repository, &["init", "-b", "main", "--quiet"]);
        fs::write(repository.join("README.md"), "main base\n").unwrap();
        git(&repository, &["add", "README.md"]);
        git(&repository, &["commit", "--quiet", "-m", "main base"]);
        git(&repository, &["checkout", "--quiet", "-b", "release"]);
        fs::write(
            repository.join("release-only.txt"),
            "catalog default branch\n",
        )
        .unwrap();
        git(&repository, &["add", "release-only.txt"]);
        git(&repository, &["commit", "--quiet", "-m", "release base"]);
        let base = git(&repository, &["rev-parse", "HEAD"]).trim().to_owned();
        git(&repository, &["checkout", "--quiet", "main"]);
        let port = free_port();
        let mut child = Command::new("git")
            .args([
                "daemon",
                "--export-all",
                "--reuseaddr",
                "--listen=127.0.0.1",
                &format!("--port={port}"),
                &format!("--base-path={}", served.display()),
            ])
            .arg(&served)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(root.join("git-daemon.log")).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "local Git daemon exited"
            );
            assert!(Instant::now() < deadline, "local Git daemon did not listen");
            std::thread::sleep(Duration::from_millis(10));
        }
        (Self(child), format!("git://127.0.0.1:{port}/repo"), base)
    }
}

fn pair(home: &Profile, target: &Profile) -> String {
    let discovery = target.discovery();
    let paired = target.cli(&["pair", "--peer"]);
    assert!(
        paired.status.success(),
        "{}",
        String::from_utf8_lossy(&paired.stderr)
    );
    let code = String::from_utf8(paired.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap()
        .to_owned();
    let linked = home.cli(&[
        "peer",
        "link",
        &format!("127.0.0.1:{}", discovery["port"]),
        &code,
    ]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    discovery["serverId"].as_str().unwrap().to_owned()
}

#[test]
fn remote_only_repository_clones_on_two_executors_and_replays_after_restart() {
    let mut home = Profile::new();
    let (_daemon, url, base) = GitDaemon::start(&home.root);
    let mut targets = [Profile::new(), Profile::new()];
    home.launch(&["serve"]);
    for target in &mut targets {
        target.launch(&["serve"]);
    }
    let servers: Vec<_> = targets.iter().map(|target| pair(&home, target)).collect();
    let device = uuid::Uuid::new_v4();
    let mut owner = home.client(device);
    hello(&mut owner);
    accepted(owner.command(
        "captain.put",
        json!({"record":{
        "name":"Repository captain","role":"Keeper","instructions":"Use the assigned repository.",
        "portrait":1}}),
        uuid::Uuid::new_v4(),
    ));
    let parent = accepted(owner.command(
        "captain.open",
        json!({"captain":0,"project":null}),
        uuid::Uuid::new_v4(),
    ))["threadId"]
        .as_u64()
        .unwrap();
    let record = accepted(owner.command(
        "repository.put",
        json!({"url":url,"baseBranch":"release","captain":0}),
        uuid::Uuid::new_v4(),
    ));
    let repository = record["id"].as_str().unwrap().to_owned();
    owner.until(|state| {
        servers.iter().all(|server| {
            state["peers"].as_array().unwrap().iter().any(|peer| {
                peer["serverId"] == *server
                    && peer["capabilities"]["models"]
                        .as_array()
                        .is_some_and(|models| models.iter().any(|model| model["id"] == "fixture"))
            })
        })
    });
    assert!(
        !home.root.join("Shina/checkouts").exists(),
        "no controller dummy clone"
    );
    let mut physical = Vec::new();
    for (index, target) in targets.iter_mut().enumerate() {
        let server = &servers[index];
        let port = target.discovery()["port"].as_u64().unwrap().to_string();
        let mut executor = target.client(uuid::Uuid::new_v4());
        hello(&mut executor);
        assert!(!target.root.join("Shina/checkouts").exists());
        let args = json!({"threadId":parent,"args":{
            "prompt":"catalog-fresh-clone-fixture","client_request_id":format!("catalog-{index}"),
            "target":{"environmentId":server,"repository":repository,"model":"fixture"}}});
        let original = accepted(owner.command("task.delegate", args.clone(), uuid::Uuid::new_v4()));
        let task_id = original["task_id"].as_str().unwrap().to_owned();
        owner.until(|state| {
            state["index"]["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|task| {
                    task["id"] == task_id
                        && matches!(
                            task["state"].as_str(),
                            Some("completed" | "failed" | "cancelled")
                        )
                })
        });
        let task = owner.snapshot["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"] == task_id)
            .unwrap()
            .clone();
        assert_eq!(task["state"], "completed", "{}", task["error_excerpt"]);
        let placement = task["remote"]["placement"].clone();
        assert_eq!(
            placement["phase"], "ready",
            "completion must carry the final placement"
        );
        assert_eq!(placement["executor"], *server);
        assert_eq!(placement["baseBranch"], "release");
        assert_eq!(placement["baseCommit"], base);
        assert_eq!(placement["binding"]["repository"], repository);
        assert_eq!(placement["binding"]["state"], "ready");
        let checkout = PathBuf::from(placement["checkout"].as_str().unwrap());
        let bound = PathBuf::from(placement["binding"]["checkout"].as_str().unwrap());
        assert_ne!(
            checkout, bound,
            "writer worktree is separate from its repository clone"
        );
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]).trim(), base);
        assert_eq!(
            fs::read_to_string(checkout.join("release-only.txt")).unwrap(),
            "catalog default branch\n"
        );
        assert_eq!(
            git(&bound, &["config", "--get", "remote.origin.url"]).trim(),
            url
        );
        let prompt_log = checkout.join("fixture-prompts.jsonl");
        assert_eq!(fs::read_to_string(&prompt_log).unwrap().lines().count(), 1);
        assert_eq!(
            fs::read_dir(target.root.join("Shina/checkouts"))
                .unwrap()
                .count(),
            1
        );
        let catalog =
            accepted(executor.command("repository.list", json!({}), uuid::Uuid::new_v4()));
        assert_eq!(catalog["repositories"][0]["id"], repository);
        assert_eq!(
            catalog["repositories"][0]["bindings"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        // The keyed delegate receipt and recorded clone/worktree survive both
        // sides restarting. Retrying the call cannot clone or prompt again.
        drop(executor);
        drop(owner);
        target.stop();
        home.stop();
        target.launch(&["serve", "--port", &port]);
        home.launch(&["serve"]);
        owner = home.client(device);
        hello(&mut owner);
        let replay = accepted(owner.command("task.delegate", args, uuid::Uuid::new_v4()));
        assert_eq!(replay["task_id"], task_id);
        assert_eq!(replay["replayed"], true);
        assert_eq!(
            fs::read_dir(target.root.join("Shina/checkouts"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(fs::read_to_string(&prompt_log).unwrap().lines().count(), 1);
        assert!(checkout.exists() && bound.exists());
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]).trim(), base);
        physical.push(bound);
    }
    assert_ne!(
        physical[0], physical[1],
        "each executor owns its own binding"
    );
    assert!(!home.root.join("Shina/checkouts").exists());
    for target in &mut targets {
        target.stop();
    }
    home.stop();
}
