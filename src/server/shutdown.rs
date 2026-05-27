//! Graceful shutdown signal handling for SIGINT (Ctrl+C) and SIGTERM.

#[cfg(unix)]
pub(super) async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for Ctrl+C");
        tracing::info!("received Ctrl+C, shutting down gracefully...");
    };

    let sigterm = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
        tracing::info!("received SIGTERM, shutting down gracefully...");
    };

    tokio::select! {
        () = sigterm => {},
        () = ctrl_c => {},
    }
}

#[cfg(not(unix))]
pub(super) async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for Ctrl+C");
    tracing::info!("received shutdown signal, shutting down gracefully...");
}
