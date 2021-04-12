use xsk_rs::{socket::*, umem::*, xsk::Xsk};

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
    env_logger::init();
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
