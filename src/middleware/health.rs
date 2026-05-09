use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::Response,
};
use tower::{Layer, Service};

const HEALTH_CHECK_HEADER: &str = "x-health-check";
const HEALTH_CHECK_VALUE: &[u8] = b"true";
const HEALTH_CHECK_BODY: &str = "OK";

pub fn is_health_check(headers: &HeaderMap) -> bool {
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
