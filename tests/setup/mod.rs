use std::sync::Once;
use xdpsock::{
    socket::*,
    umem::*,
    xsk::{Xsk, Xsk2, Xsk2Config},
};

static INIT: Once = Once::new();

fn setup_logging() {
    INIT.call_once(|| {
        env_logger::init();
    });
}

mod veth_setup;

pub fn run_test<F>(
    dev1_umem_config: Option<UmemConfig>,
    dev1_socket_config: Option<SocketConfig>,
    dev2_umem_config: Option<UmemConfig>,
    dev2_socket_config: Option<SocketConfig>,
    test: F,
) where
    F: Fn(Xsk, Xsk) + Send + 'static,
{
    setup_logging();
    let inner = move |dev1_if_name: String, dev2_if_name: String| {
        // Create the socket for the first interfaace

        let dev1_umem_config = dev1_umem_config.unwrap_or_default();
        let dev2_umem_config = dev2_umem_config.unwrap_or_default();
        let dev1_socket_config = dev1_socket_config.unwrap_or_default();
        let dev2_socket_config = dev2_socket_config.unwrap_or_default();

        let dev1_n_tx_frames = dev1_umem_config.frame_count() / 2;
        let dev2_n_tx_frames = dev2_umem_config.frame_count() / 2;
        let xsk1 = Xsk::new(
            &dev1_if_name,
            0,
            dev1_umem_config,
            dev1_socket_config,
            dev1_n_tx_frames as usize,
        );
        let xsk2 = Xsk::new(
            &dev2_if_name,
            0,
            dev2_umem_config,
            dev2_socket_config,
            dev2_n_tx_frames as usize,
        );

        test(xsk1, xsk2)
    };

    veth_setup::run_with_dev(inner);
}

pub fn run_test_2<F>(mut dev1_xsk_config: Xsk2Config, mut dev2_xsk_config: Xsk2Config, test: F)
where
    F: Fn(Xsk2, Xsk2) + Send + 'static,
{
    setup_logging();
    let inner = move |dev1_if_name: String, dev2_if_name: String| {
        // Create the socket for the first interfaace
        dev1_xsk_config.if_name = dev1_if_name;
        dev2_xsk_config.if_name = dev2_if_name;
        let xsk1 = Xsk2::from_config(dev1_xsk_config).expect("failed to build xsk2");
        let xsk2 = Xsk2::from_config(dev2_xsk_config).expect("failed to build xsk2");

        test(xsk1, xsk2)
    };

    veth_setup::run_with_dev(inner);
}
