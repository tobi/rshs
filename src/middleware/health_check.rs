use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use actix_web::{
    HttpResponse,
    body::{EitherBody, MessageBody},
    dev::{self, ServiceRequest, ServiceResponse},
};

const HEALTH_CHECK_HEADER: &str = "x-health-check";
const HEALTH_CHECK_VALUE: &[u8] = b"true";
const HEALTH_CHECK_BODY: &str = "OK";

pub fn is_health_check(headers: &actix_web::http::header::HeaderMap) -> bool {
    headers.get(HEALTH_CHECK_HEADER).map(|v| v.as_bytes()) == Some(HEALTH_CHECK_VALUE)
}

#[derive(Clone)]
pub struct HealthCheck;

impl<S, B> dev::Transform<S, ServiceRequest> for HealthCheck
where
    S: dev::Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>
        + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = actix_web::Error;
    type InitError = ();
    type Transform = HealthCheckService<S>;
    type Future = std::future::Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        std::future::ready(Ok(HealthCheckService { service }))
    }
}

pub struct HealthCheckService<S> {
    service: S,
}

impl<S, B> dev::Service<ServiceRequest> for HealthCheckService<S>
where
    S: dev::Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>
        + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = actix_web::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        if is_health_check(req.headers()) {
            let peer = req
                .connection_info()
                .peer_addr()
                .unwrap_or("unknown")
                .to_owned();
            let (http_req, _) = req.into_parts();
            tracing::debug!(%peer, "health check");
            let response = HttpResponse::Ok()
                .content_type("text/plain; charset=utf-8")
                .body(HEALTH_CHECK_BODY);
            let svc_resp = ServiceResponse::new(http_req, response).map_into_right_body();
            Box::pin(std::future::ready(Ok(svc_resp)))
        } else {
            let fut = self.service.call(req);
            Box::pin(async move { fut.await.map(|res| res.map_into_left_body()) })
        }
    }
}
