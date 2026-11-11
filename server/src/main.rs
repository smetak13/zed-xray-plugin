use tower_lsp_server::{LspService, Server};

#[tokio::main]
async fn main() {
    let (service, socket) = LspService::new(xray::server::Server::new);
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
}
