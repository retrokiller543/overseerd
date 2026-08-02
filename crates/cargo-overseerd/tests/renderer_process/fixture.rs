use std::collections::BTreeMap;

use overseerd_tooling_schema::TOOLING_SCHEMA_VERSION;
use overseerd_tooling_schema::renderer::{
    RendererRequest, RendererResponse, ResourcePresentation, TOOLING_RENDERER_ARGUMENT,
    TOOLING_RENDERER_REQUEST_ENV, TOOLING_RENDERER_RESPONSE_ENV,
};

fn main() {
    let mut arguments = std::env::args();
    let _binary = arguments.next();

    if arguments.next().as_deref() != Some(TOOLING_RENDERER_ARGUMENT) {
        std::process::exit(2);
    }

    let request_path =
        std::env::var_os(TOOLING_RENDERER_REQUEST_ENV).expect("renderer request path is present");
    let response_path =
        std::env::var_os(TOOLING_RENDERER_RESPONSE_ENV).expect("renderer response path is present");
    let request_json = std::fs::read_to_string(request_path).expect("renderer request is readable");
    let request = RendererRequest::from_json(&request_json).expect("renderer request validates");
    let resources = request
        .document
        .resources
        .iter()
        .filter(|resource| {
            resource.id == request.owner
                || resource
                    .provenance
                    .as_ref()
                    .and_then(|provenance| provenance.owner.as_deref())
                    == Some(request.owner.as_str())
        })
        .map(|resource| ResourcePresentation {
            resource: resource.id.clone(),
            label: Some(format!("rendered {}", resource.name)),
            group: Some(String::from("fixture group")),
            summary: Some(String::from("fixture summary")),
            details: BTreeMap::from([(String::from("fixture"), String::from("active"))]),
        })
        .collect();
    let response = RendererResponse {
        schema: TOOLING_SCHEMA_VERSION,
        renderer: request.renderer.clone(),
        owner: request.owner.clone(),
        presentation: overseerd_tooling_schema::renderer::RendererPresentation { resources },
    };
    let response_json = response
        .to_json(&request)
        .expect("renderer response validates");

    std::fs::write(response_path, response_json).expect("renderer response is written");
}
