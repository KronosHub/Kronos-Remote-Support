//! The file manager between two real WebRTC peers (LAN signaling, no server): the viewer
//! lists the host's folder, creates one, copies files both ways and cancels a copy.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use signaling_proto::PeerId;
use stream::fs::FsClient;
use transport::{PeerSession, Role, lan, sort_channels};

async fn pump(
    mut peer: PeerSession,
    mut incoming: tokio::sync::mpsc::UnboundedReceiver<signaling_proto::SignalPayload>,
    done: tokio::sync::oneshot::Receiver<()>,
) {
    tokio::pin!(done);
    loop {
        tokio::select! {
            p = incoming.recv() => match p {
                Some(p) => { let _ = peer.handle_signal(p).await; }
                None => break,
            },
            _ = &mut done => break,
        }
    }
    peer.close().await;
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kronos-fm-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn browse_and_copy_both_ways() {
    let (port, mut incoming) = lan::listen().await.unwrap();
    let viewer_id: PeerId = "123456789".parse().unwrap();
    let host_id: PeerId = "987654321".parse().unwrap();
    let viewer_lan = lan::connect(
        &[SocketAddr::from(([127, 0, 0, 1], port))],
        &viewer_id,
        &host_id,
    )
    .await
    .unwrap();
    let host_lan = incoming.recv().await.unwrap();
    let mut host = PeerSession::start(
        Role::Answerer,
        host_lan.session,
        &[],
        false,
        host_lan.signal,
    )
    .await
    .unwrap();
    let mut viewer = PeerSession::start(
        Role::Offerer,
        viewer_lan.session,
        &[],
        false,
        viewer_lan.signal,
    )
    .await
    .unwrap();
    let host_channels = host.take_data_channels().unwrap();
    let viewer_channels = viewer.take_data_channels().unwrap();
    let (host_done_tx, host_done) = tokio::sync::oneshot::channel();
    let (viewer_done_tx, viewer_done) = tokio::sync::oneshot::channel();
    let h = tokio::spawn(pump(host, host_lan.incoming, host_done));
    let v = tokio::spawn(pump(viewer, viewer_lan.incoming, viewer_done));

    let timeout = Duration::from_secs(30);
    let (host_fs, viewer_fs) = tokio::time::timeout(timeout, async {
        let host = sort_channels(host_channels).await.unwrap();
        let viewer = sort_channels(viewer_channels).await.unwrap();
        (host.fs.await.unwrap(), viewer.fs.await.unwrap())
    })
    .await
    .expect("fs channels");
    let (log_tx, mut logs) = tokio::sync::mpsc::unbounded_channel();
    let server = tokio::spawn(stream::fs::serve(host_fs, move |m| {
        let _ = log_tx.send(m);
    }));
    let client = FsClient::new(viewer_fs);

    let host_dir = temp_dir("host");
    let viewer_dir = temp_dir("viewer");
    std::fs::write(host_dir.join("report.txt"), "dal computer controllato").unwrap();
    let big = vec![7u8; 3 * 1024 * 1024 + 123];
    std::fs::create_dir(viewer_dir.join("progetto")).unwrap();
    std::fs::write(viewer_dir.join("progetto/dati.bin"), &big).unwrap();

    tokio::time::timeout(timeout, async {
        // The host's home folder by default, any folder on request.
        let home = client.list(None).await.unwrap();
        assert_eq!(Some(home.path.clone()), home.home);
        let listing = client.list(Some(s(&host_dir))).await.unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert!(client.list(Some(s(&host_dir.join("manca")))).await.is_err());

        client.make_dir(s(&host_dir.join("nuova"))).await.unwrap();
        assert!(host_dir.join("nuova").is_dir());

        // Viewer → host: a folder with a 3 MB file, with progress.
        let (progress_tx, mut progress) = tokio::sync::mpsc::unbounded_channel();
        let req = client.new_request();
        let names = client
            .upload(
                req,
                vec![viewer_dir.join("progetto")],
                s(&host_dir),
                false,
                Some(progress_tx),
            )
            .await
            .unwrap();
        assert_eq!(names, ["progetto"]);
        assert_eq!(
            std::fs::read(host_dir.join("progetto/dati.bin")).unwrap(),
            big
        );
        let mut last = None;
        while let Ok(p) = progress.try_recv() {
            last = Some(p);
        }
        let last = last.expect("progress reported");
        assert!(last.finished && last.sending && last.done == big.len() as u64);

        // Host → viewer, twice: the second copy gets a new name.
        for expected in ["report.txt", "report (2).txt"] {
            let req = client.new_request();
            let names = client
                .download(
                    req,
                    vec![s(&host_dir.join("report.txt"))],
                    viewer_dir.clone(),
                    false,
                    None,
                )
                .await
                .unwrap();
            assert_eq!(names, [expected]);
        }
        assert_eq!(
            std::fs::read_to_string(viewer_dir.join("report (2).txt")).unwrap(),
            "dal computer controllato"
        );
        // Overwriting: same name, new content, both ways.
        std::fs::write(viewer_dir.join("report.txt"), "vecchio").unwrap();
        let req = client.new_request();
        let names = client
            .download(
                req,
                vec![s(&host_dir.join("report.txt"))],
                viewer_dir.clone(),
                true,
                None,
            )
            .await
            .unwrap();
        assert_eq!(names, ["report.txt"]);
        assert_eq!(
            std::fs::read_to_string(viewer_dir.join("report.txt")).unwrap(),
            "dal computer controllato"
        );
        std::fs::write(viewer_dir.join("progetto/dati.bin"), b"altro").unwrap();
        let req = client.new_request();
        let names = client
            .upload(
                req,
                vec![viewer_dir.join("progetto")],
                s(&host_dir),
                true,
                None,
            )
            .await
            .unwrap();
        assert_eq!(names, ["progetto"]);
        assert_eq!(
            std::fs::read(host_dir.join("progetto/dati.bin")).unwrap(),
            b"altro"
        );
        // A missing file: an error, not a hang.
        let req = client.new_request();
        assert!(
            client
                .download(
                    req,
                    vec![s(&host_dir.join("manca.txt"))],
                    viewer_dir.clone(),
                    false,
                    None
                )
                .await
                .is_err()
        );
    })
    .await
    .expect("file manager operations");

    // A cancelled download leaves nothing behind on the viewer.
    std::fs::write(host_dir.join("grande.bin"), vec![1u8; 40 * 1024 * 1024]).unwrap();
    let dest = temp_dir("cancel");
    let client = std::sync::Arc::new(client);
    let req = client.new_request();
    let copy = tokio::spawn({
        let client = client.clone();
        let source = s(&host_dir.join("grande.bin"));
        let dest = dest.clone();
        async move { client.download(req, vec![source], dest, false, None).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    client.cancel(req).await;
    let result = tokio::time::timeout(timeout, copy).await.unwrap().unwrap();
    assert!(result.is_err());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        std::fs::read_dir(&dest).unwrap().count(),
        0,
        "nothing left after cancel"
    );
    // The channel still works after the cancel.
    assert!(client.list(Some(s(&host_dir))).await.is_ok());

    let mut messages = Vec::new();
    while let Ok(m) = logs.try_recv() {
        messages.push(m);
    }
    assert!(messages.iter().any(|m| m.starts_with("sta sfogliando")));
    assert!(messages.iter().any(|m| m.contains("ha copiato 1 elemento")));

    server.abort();
    for dir in [host_dir, viewer_dir, dest] {
        let _ = std::fs::remove_dir_all(dir);
    }
    let _ = host_done_tx.send(());
    let _ = viewer_done_tx.send(());
    let _ = tokio::join!(h, v);
}
