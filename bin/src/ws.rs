use async_trait::async_trait;
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};

pub(crate) struct RequestInterceptor<D: RequestHandler> {
    downstream: D
}

impl<D: RequestHandler> RequestInterceptor<D> {
    pub(crate) fn new(downstream: D) -> Self {
        Self { downstream }
    }
}

#[async_trait]
impl<D: RequestHandler> RequestHandler for RequestInterceptor<D> {
    async fn handle_request<R: ResponseHandler>(&self, request: &Request, response_handle: R) -> ResponseInfo {
        println!("intercepted request from {}: {} {} {}", request.src(), request.query().query_class(), request.query().query_type(), request.query().name());
        self.downstream.handle_request(request, response_handle).await
    }
}

