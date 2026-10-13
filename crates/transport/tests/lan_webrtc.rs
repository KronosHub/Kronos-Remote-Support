//! Two real WebRTC peers connected through the LAN signaling link only (no server): the
//! offer/answer/ICE exchange goes over the direct TCP connection and the data channel opens.

use std::net::SocketAddr;
use std::time::Duration;

use signaling_proto::PeerId;
use transport::{PeerSession, Role, diag, lan};

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

#[tokio::test(flavor = "multi_thread")]
async fn webrtc_over_lan_signaling() {
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
    let mut host_channels = host.take_data_channels().unwrap();
    let mut viewer_channels = viewer.take_data_channels().unwrap();

    let (host_done_tx, host_done) = tokio::sync::oneshot::channel();
    let (viewer_done_tx, viewer_done) = tokio::sync::oneshot::channel();
    let h = tokio::spawn(pump(host, host_lan.incoming, host_done));
    let v = tokio::spawn(pump(viewer, viewer_lan.incoming, viewer_done));

    let opened = tokio::time::timeout(Duration::from_secs(15), async {
        let host_control = host_channels.recv().await.expect("host gets a channel");
        let viewer_control = viewer_channels.recv().await.expect("viewer has a channel");
        diag::wait_open(&viewer_control).await.unwrap();
        diag::wait_open(&host_control).await.unwrap();
    })
    .await;
    assert!(
        opened.is_ok(),
        "data channel did not open over LAN signaling"
    );

    let _ = host_done_tx.send(());
    let _ = viewer_done_tx.send(());
    let _ = tokio::join!(h, v);
}
