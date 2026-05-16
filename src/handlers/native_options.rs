use axum::{body::Body, http::StatusCode, response::Response};

pub async fn handle() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(
            "allow",
            "GET, HEAD, OPTIONS, PUT, DELETE, PROPFIND, MKCOL, COPY, MOVE, PROPPATCH, LOCK, UNLOCK",
        )
        .header("content-length", "0")
        .body(Body::empty())
        .unwrap()
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_options_returns_ok() {
        let resp = super::handle().await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let allow = resp.headers().get("allow").unwrap().to_str().unwrap();
        assert!(allow.contains("GET"));
        assert!(allow.contains("PUT"));
        assert!(allow.contains("DELETE"));
        assert!(allow.contains("PROPFIND"));
        assert!(allow.contains("MKCOL"));
    }

    #[tokio::test]
    async fn test_options_has_content_length_zero() {
        let resp = super::handle().await;
        let cl = resp
            .headers()
            .get("content-length")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(cl, "0");
    }

    #[tokio::test]
    async fn test_options_body_empty() {
        let resp = super::handle().await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(body.is_empty());
    }
}
