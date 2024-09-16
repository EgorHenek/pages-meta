use axum::{extract::Request, http::HeaderValue, middleware::Next, response::Response};
use tokio::time::Instant;

#[derive(Clone, Copy)]
struct RequestStartTime(Instant);

pub async fn timing_middleware(mut request: Request, next: Next) -> Response {
    let start = Instant::now();

    request.extensions_mut().insert(RequestStartTime(start));

    let start_time = *request.extensions().get::<RequestStartTime>().unwrap();

    let mut response = next.run(request).await;

    let duration = start_time.0.elapsed();

    response.headers_mut().insert(
        "X-Response-Time",
        HeaderValue::from_str(&format!("{:.2?}", duration)).unwrap(),
    );

    response
}
