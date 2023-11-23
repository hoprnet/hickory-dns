use async_trait::async_trait;
use futures_channel::mpsc::UnboundedSender;
use futures_util::lock::Mutex;
use futures_util::StreamExt;
use hickory_proto::error::ProtoError;
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info};

pub(crate) struct RequestInterceptor<D: RequestHandler> {
    downstream: D,
    ws: Arc<WsServer>,
}

impl<D: RequestHandler> RequestInterceptor<D> {
    pub(crate) fn new(downstream: D, ws: Arc<WsServer>) -> Self {
        Self { downstream, ws }
    }
}

#[async_trait]
impl<D: RequestHandler> RequestHandler for RequestInterceptor<D> {
    async fn handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        response_handle: R,
    ) -> ResponseInfo {
        let qry = request.query();
        let msg = format!(
            "[{}, \"{}\", \"{}\", \"{}\", \"{}\"]",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
            request.src().ip(),
            qry.query_class(),
            qry.query_type(),
            qry.name()
        );

        debug!("intercepted request from {msg}");
        self.ws.send_message(request.src().ip(), msg).await;

        self.downstream
            .handle_request(request, response_handle)
            .await
    }
}

type PeerMap = HashMap<String, UnboundedSender<String>>;

pub(crate) struct WsServer {
    connections: Arc<Mutex<PeerMap>>,
}

impl WsServer {
    pub(crate) fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn send_message(&self, to: IpAddr, message: String) {
        let to_str = to.to_string();

        let mut conns = self.connections.lock().await;
        if let Some(sender) = conns.get(&to_str) {
            match sender.unbounded_send(message) {
                Ok(_) => {
                    debug!("message sent to {to}")
                }
                Err(e) => {
                    error!("failed to deliver message to {to}");
                    if e.is_disconnected() {
                        conns.remove(&to_str);
                    }
                }
            }
        }
    }

    pub(crate) async fn spawn(&self, bound_socket: TcpListener) -> Result<(), ProtoError> {
        info!("WS listening on {}", bound_socket.local_addr().unwrap());

        while let Ok((stream, addr)) = bound_socket.accept().await {
            self.accept(addr, stream)
                .await
                .map_err(|e| std::io::Error::new(ErrorKind::Other, e))?;
        }

        debug!("WS listening done");
        Ok(())
    }

    async fn accept(
        &self,
        addr: SocketAddr,
        stream: TcpStream,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        let ws_stream = tokio_tungstenite::accept_async(stream).await?;
        let (write, read) = ws_stream.split();

        let (tx, rx) = futures_channel::mpsc::unbounded();
        self.connections
            .lock()
            .await
            .insert(addr.ip().to_string(), tx);
        debug!("WS peer {addr} connected");

        let peer_map = self.connections.clone();
        tokio::spawn(async move {
            let _ = rx.map(|s| Ok(Message::Text(s))).forward(write).await;

            debug!("WS peer {addr} disconnected");
            peer_map.lock().await.remove(&addr.ip().to_string());
            std::mem::drop(read);
        });

        Ok(())
    }
}
