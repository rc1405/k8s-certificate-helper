use kube::core::{
    admission::{AdmissionRequest, AdmissionResponse, AdmissionReview},
    DynamicObject,
};
use kube::Client;
use serde_json::Value;
use std::convert::{From, Infallible};
use tracing::info;
use warp::{reply, Filter, Reply};

use crate::controller::Error;
use crate::crd::Certificate;

pub async fn serve(port: u16) -> Result<(), Error> {
    let client = Client::try_default().await?;

    let routes = warp::path("validate")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(move |body: AdmissionReview<DynamicObject>| handler(client.clone(), body))
        .with(warp::trace::request());

    warp::serve(warp::post().and(routes))
        .tls()
        .cert_path("/webhook-helper/tls.crt")
        .key_path("/webhook-helper/tls.key")
        .run(([0, 0, 0, 0], port))
        .await;

    Ok(())
}

#[allow(unused_variables)]
async fn handler(
    client: Client,
    body: AdmissionReview<DynamicObject>,
) -> Result<impl Reply, Infallible> {
    // Parse incoming webhook AdmissionRequest first
    let req: AdmissionRequest<_> = match body.try_into() {
        Ok(req) => req,
        Err(err) => {
            return Ok(reply::json(
                &AdmissionResponse::invalid(err.to_string()).into_review(),
            ));
        }
    };

    let mut res = AdmissionResponse::from(&req);
    let raw: Value = match req.object {
        Some(o) => {
            if let Ok(r) = serde_json::to_value(o) {
                r
            } else {
                res = res.deny("invalid request format".to_string().to_string());
                return Ok(reply::json(&res.into_review()));
            }
        }
        None => return Ok(reply::json(&res.into_review())),
    };

    let resource: Certificate = match serde_json::from_value(raw) {
        Ok(v) => v,
        Err(_) => {
            res = res.deny("invalid request format".to_string().to_string());
            return Ok(reply::json(&res.into_review()));
        }
    };

    info!("Certificate helper validated");

    // Wrap the AdmissionResponse wrapped in an AdmissionReview
    Ok(reply::json(&res.into_review()))
}
