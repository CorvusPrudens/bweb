use std::time::Duration;

pub async fn sleep(duration: Duration) {
    gloo_timers::future::sleep(duration).await
}
