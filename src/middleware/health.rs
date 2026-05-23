//! Health check middleware — intercepts `x-health-check: true` before auth.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use tower::{Layer, Service};

const HEALTH_CHECK_HEADER: &str = "x-health-check";
const HEALTH_CHECK_VALUE: &[u8] = b"true";
const HEALTH_CHECK_BODY: &str = "OK";

pub(crate) fn is_health_check(headers: &HeaderMap) -> bool {
    headers.get(HEALTH_CHECK_HEADER).map(|v| v.as_bytes()) == Some(HEALTH_CHECK_VALUE)
}

#[derive(Clone)]
pub struct HealthCheck;

impl<S> Layer<S> for HealthCheck {
    type Service = HealthCheckService<S>;

    fn layer(&self, service: S) -> Self::Service {
        HealthCheckService { service }
    }
}

#[derive(Clone)]
pub struct HealthCheckService<S> {
    service: S,
}

impl<S> Service<axum::extract::Request> for HealthCheckService<S>
where
    S: Service<axum::extract::Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send,
{
    type Response = Response;
    type Error = S::Error;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: axum::extract::Request) -> Self::Future {
        if is_health_check(req.headers()) {
            let response = Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/plain; charset=utf-8")
                .body(Body::from(HEALTH_CHECK_BODY))
                .unwrap();
            Box::pin(std::future::ready(Ok(response)))
        } else {
            let mut svc = self.service.clone();
            Box::pin(async move { svc.call(req).await })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{Router, body::Body, extract::Request};
    use tower::ServiceExt;

    use crate::handlers::http::handle_get_head;
    use crate::{AppState, AuthConfig};

    fn setup_health_test_dir() -> tempfile::TempDir {
        use std::io::Write;
        let dir = tempfile::TempDir::new().unwrap();
        let mut f = std::fs::File::create(dir.path().join("hello.txt")).unwrap();
        f.write_all(b"Hello, World!").unwrap();
        dir
    }

    fn make_app_health(dir: &tempfile::TempDir) -> Router {
        Router::new()
            .fallback(handle_get_head)
            .layer(super::HealthCheck)
            .with_state(Arc::new(AppState::new(
                dir.path().to_path_buf(),
                AuthConfig::new(),
                std::time::Duration::from_secs(300),
            )))
    }

    #[tokio::test]
    async fn test_health_check_returns_ok() {
        let dir = setup_health_test_dir();
        let app = make_app_health(&dir);

        let req = Request::builder()
            .method(axum::http::Method::GET)
            .uri("/")
            .header("x-health-check", "true")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"OK");
    }

    #[tokio::test]
    async fn test_health_check_content_type() {
        let dir = setup_health_test_dir();
        let app = make_app_health(&dir);

        let req = Request::builder()
            .method(axum::http::Method::GET)
            .uri("/")
            .header("x-health-check", "true")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert!(
            resp.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("text/plain")
        );
    }

    #[tokio::test]
    async fn test_health_check_without_header_passes_through() {
        let dir = setup_health_test_dir();
        let app = make_app_health(&dir);

        let req = Request::builder()
            .method(axum::http::Method::GET)
            .uri("/hello.txt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert!(resp.status().is_success());

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"Hello, World!");
    }

    #[tokio::test]
    async fn test_health_check_with_wrong_header_value_passes_through() {
        let dir = setup_health_test_dir();
        let app = make_app_health(&dir);

        let req = Request::builder()
            .method(axum::http::Method::GET)
            .uri("/hello.txt")
            .header("x-health-check", "false")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert!(resp.status().is_success());

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"Hello, World!");
    }

    #[tokio::test]
    async fn test_health_check_with_head_method() {
        let dir = setup_health_test_dir();
        let app = make_app_health(&dir);

        let req = Request::builder()
            .method(axum::http::Method::HEAD)
            .uri("/")
            .header("x-health-check", "true")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_is_health_check_function() {
        use axum::http::header::{HeaderMap, HeaderName, HeaderValue};

        let mut headers = HeaderMap::new();
        assert!(!super::is_health_check(&headers));

        headers.insert(
            HeaderName::from_static("x-health-check"),
            HeaderValue::from_static("true"),
        );
        assert!(super::is_health_check(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-health-check"),
            HeaderValue::from_static("false"),
        );
        assert!(!super::is_health_check(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-health-check"),
            HeaderValue::from_static("1"),
        );
        assert!(!super::is_health_check(&headers));
    }
}
