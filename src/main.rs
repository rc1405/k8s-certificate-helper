use clap::{Args, Parser};
use futures::join;

mod admission;
mod controller;
mod crd;
mod operator;

#[derive(Parser)]
#[command(name = "certificate-helper")]
#[command(bin_name = "certificate-helper")]
enum CertificateHelperCli {
    Run(RunArgs),
}

#[derive(Args)]
#[command(author, version, about, long_about = None)]
pub struct RunArgs {
    #[arg(short, long)]
    port: u16,
}

/// something to drive the controller
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    match CertificateHelperCli::parse() {
        CertificateHelperCli::Run(args) => {
            let adm_proc = admission::serve(args.port);
            let controller_proc = controller::run();
            let (adm_result, controller_result) = join!(adm_proc, controller_proc);
            adm_result?;
            controller_result?;
        }
    };

    Ok(())
}
