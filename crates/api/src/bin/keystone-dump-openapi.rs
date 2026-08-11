//! Print the OpenAPI JSON document to stdout.
//!
//! No database, no configuration, no server — the document is generated at
//! compile time from the `#[utoipa::path]` annotations. This is the single
//! source for the generated frontend client:
//!
//! ```sh
//! cargo run -p keystone-api --bin keystone-dump-openapi > web/openapi.json
//! ```

use keystone_api::openapi::ApiDoc;
use utoipa::OpenApi;

fn main() {
    let spec = ApiDoc::openapi()
        .to_pretty_json()
        .expect("OpenAPI document must serialize");
    println!("{spec}");
}
