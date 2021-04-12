use rand::{distributions::Alphanumeric, thread_rng, Rng};
use veth_util_rs::{add_veth_link, VethConfig};

pub fn run_with_dev<F>(f: F)
where
    F: FnOnce(String, String) + Send + 'static,
{
    let rand_string: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();

    let dev1_ifname = format!("xsk1_{}", rand_string);
    let dev2_ifname = format!("xsk2_{}", rand_string);

    let veth_config = VethConfig::new(dev1_ifname, dev2_ifname);
    let veth_pair = add_veth_link(&veth_config).expect("failed to build veth pair");

    f(
        veth_pair.dev1().ifname().into(),
        veth_pair.dev2().ifname().into(),
    );
}
