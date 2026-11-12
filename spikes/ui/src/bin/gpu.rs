use std::time::Instant;
fn main() {
    let t = Instant::now();
    let instance = wgpu::Instance::default();
    let adapters = block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    println!("wgpu instance+enumerate {:?}", t.elapsed());
    for a in adapters {
        let i = a.get_info();
        println!("  adapter: {} ({:?}, {:?})", i.name, i.backend, i.device_type);
    }
    let _ = winit::event_loop::EventLoop::<()>::new().map(|_| println!("winit event loop ok"));
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll, Wake, Waker};
    struct N;
    impl Wake for N {
        fn wake(self: std::sync::Arc<Self>) {}
    }
    let w = Waker::from(std::sync::Arc::new(N));
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut Context::from_waker(&w)) {
            return v;
        }
        std::thread::yield_now();
    }
}
