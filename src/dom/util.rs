use crate::js_err::JsErr;
use crate::prelude::*;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use std::borrow::Cow;

pub(crate) struct UtilsPlugin;

impl Plugin for UtilsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(SetTitle::observe);
    }
}

/// A `blob:` URL backed by in-memory bytes and revoked on drop.
pub struct ObjectUrl(String);

impl ObjectUrl {
    /// Wrap `bytes` in a `Blob` of the given mime type and register an object
    /// url for it.
    pub fn new(bytes: &[u8], mime: &str) -> Result<Self> {
        let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
        array.copy_from(bytes);

        let props = web_sys::BlobPropertyBag::new();
        props.set_type(mime);

        let parts = js_sys::Array::of1(&array.buffer());
        let blob =
            web_sys::Blob::new_with_buffer_source_sequence_and_options(&parts, &props).js_err()?;

        Ok(Self(
            web_sys::Url::create_object_url_with_blob(&blob).js_err()?,
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for ObjectUrl {
    fn drop(&mut self) {
        let _ = web_sys::Url::revoke_object_url(&self.0);
    }
}

#[derive(Event)]
pub struct SetTitle(pub Cow<'static, str>);

impl SetTitle {
    pub fn new(title: impl Into<Cow<'static, str>>) -> Self {
        Self(title.into())
    }

    fn observe(
        ev: On<Self>,
        head: Single<Entity, With<Head>>,
        title: Query<Entity, With<Title>>,
        document: Single<&Document>,
        mut commands: Commands,
    ) {
        let value = ev.0.clone();

        if let Ok(title) = title.single() {
            commands
                .entity(title)
                .despawn_children()
                .with_child(Text::new(value));
        } else {
            // clean up title elements that weren't placed by bweb, since they
            // would otherwise hog the title value
            while let Some(stale) = document.query_selector("head > title").ok().flatten() {
                stale.remove();
            }

            commands.spawn((ChildOf(*head), Title, children![Text::new(value)]));
        }
    }
}
