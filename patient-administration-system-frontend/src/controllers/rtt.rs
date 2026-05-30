//! RTT cockpit controller — one page at `/rtt`.

use loco_rs::prelude::*;
use serde_json::json;

use crate::models::rtt::list_active_pathways;

pub fn routes() -> Routes {
    Routes::new().prefix("/rtt").add("/", get(index))
}

/// `GET /rtt` — every non-stopped RTT pathway with computed weeks-waiting,
/// sorted worst-breach-first.
pub async fn index(
    ViewEngine(v): ViewEngine<TeraView>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let pathways = list_active_pathways(&ctx.db).await.map_err(Error::wrap)?;
    let breach_count = pathways.iter().filter(|p| p.is_breaching).count();
    format::render().view(
        &v,
        "rtt/index.html",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "pathways": pathways,
            "total_pathways": pathways.len(),
            "breach_count": breach_count,
        }),
    )
}
