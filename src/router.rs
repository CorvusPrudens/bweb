use std::sync::Arc;

use crate::dom::{DomStartupSystems, prelude::*};
use crate::dom::{DomSystems, events::Events};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::component::HookContext;
use bevy_ecs::prelude::*;
use bevy_ecs::world::DeferredWorld;
use bevy_log::info;
use wasm_bindgen::JsValue;

// TODO: okay this should probably be a lil entity set guy
pub(super) struct RouterPlugin;

impl Plugin for RouterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreStartup,
            initialize_router.in_set(DomStartupSystems::Pathname),
        )
        .add_systems(
            PostUpdate,
            (
                resolve_routes.in_set(DomSystems::ResolveRoutes),
                hook_into_anchors
                    .after(DomSystems::Reparent)
                    .before(DomSystems::Attach),
            ),
        );

        #[cfg(debug_assertions)]
        app.add_systems(
            PostUpdate,
            (|pathname: Res<Pathname>| {
                info!("navigation: {pathname:?}");
            })
            .run_if(resource_changed::<Pathname>),
        );
    }
}

#[derive(Debug, Resource)]
pub struct Pathname {
    previous_path: Option<String>,
    pathname: String,
}

impl Pathname {
    pub fn pathname(&self) -> &str {
        &self.pathname
    }

    pub fn update(&mut self, mut new_pathname: String) {
        core::mem::swap(&mut self.pathname, &mut new_pathname);
        self.previous_path = Some(new_pathname);
    }
}

fn initialize_router(window: Single<(Entity, &Window)>, mut commands: Commands) -> Result {
    let (window_entity, window) = window.into_inner();

    commands.spawn((
        EventOf(window_entity),
        OnPopState::new(
            |_: Ev<web_sys::PopStateEvent>,
             window: Single<&Window>,
             mut pathname: ResMut<Pathname>| {
                let location = window.location();
                let base = location.href().unwrap();
                let url = web_sys::Url::new(&base).unwrap();
                let new_pathname = url.pathname();

                pathname.update(new_pathname);
            },
        ),
    ));

    let location = window.location();
    let base = location.href().js_err()?;
    let url = web_sys::Url::new(&base).js_err()?;
    let pathname = url.pathname();

    commands.insert_resource(Pathname {
        pathname,
        previous_path: None,
    });

    Ok(())
}

#[derive(Component)]
struct RouterLink;

fn hook_into_anchors(
    anchors: Query<
        (
            Entity,
            &Href,
            Option<&Events>,
            Has<Download>,
            Option<&Target>,
        ),
        (With<A>, Changed<Href>),
    >,
    events: Query<Entity, With<RouterLink>>,
    window: Single<&Window>,
    mut commands: Commands,
) -> Result {
    let location = window.location();
    let base = location.href().js_err()?;

    for (entity, href, handlers, has_download, target) in &anchors {
        let absolute = web_sys::Url::new(href).is_ok();
        if absolute || has_download || target.is_some_and(|t| t.as_ref() == "_blank") {
            // if absolute, remove potential dangling handler
            if let Some(handler) = handlers
                .iter()
                .flat_map(|h| h.iter())
                .find_map(|h| events.get(h).ok())
            {
                commands.entity(handler).despawn();
            }

            // no need to intercept
            continue;
        }

        let url = web_sys::Url::new_with_base(href, &base).js_err()?;
        let href = url.href();
        let path = url.pathname();

        commands.spawn((
            RouterLink,
            EventOf(entity),
            OnClick::new(
                move |ev: Ev<web_sys::PointerEvent>,
                      window: Single<&Window>,
                      mut pathname: ResMut<Pathname>| {
                    if ev.ctrl_key() || ev.meta_key() {
                        return;
                    }

                    ev.prevent_default();
                    window
                        .history()
                        .unwrap()
                        .push_state_with_url(&JsValue::NULL, "", Some(&href))
                        .unwrap();

                    pathname.update(path.clone());
                },
            ),
        ));
    }

    Ok(())
}

#[derive(Clone)]
struct RouteElement(Arc<dyn Fn(&mut EntityWorldMut) + Send + Sync>);

#[derive(Component, Default)]
pub struct Route {
    routes: Vec<(String, RouteElement)>,
}

#[derive(Component)]
struct MatchedRoute(String);

impl Route {
    pub fn push<F, B>(&mut self, route: &str, element: F) -> &mut Self
    where
        F: Fn() -> B + Send + Sync + 'static,
        B: Bundle,
    {
        let element = RouteElement(Arc::new(move |commands: &mut EntityWorldMut| {
            commands.insert(element());
        }));

        self.routes.push((route.into(), element));
        self
    }
}

fn resolve_routes(
    body: Query<Entity, With<Body>>,
    nodes: Query<(Option<&Route>, Option<&MatchedRoute>, Option<&Children>)>,
    pathname: Res<Pathname>,
    mut commands: Commands,
) -> Result {
    fn find_routes(
        nodes: &Query<(Option<&Route>, Option<&MatchedRoute>, Option<&Children>)>,
        parent_entity: Entity,
        path: &mut String,
        commands: &mut Commands,
    ) -> Result {
        let (.., children) = nodes.get(parent_entity)?;

        for child_entity in children.iter().flat_map(|c| c.iter()) {
            let Ok((child_route, matched_route, ..)) = nodes.get(child_entity) else {
                continue;
            };

            let mut temp_path;
            let path = match child_route {
                Some(route) => {
                    let best = route
                        .routes
                        .iter()
                        .filter(|(route, ..)| path.starts_with(route))
                        .max_by_key(|(route, ..)| {
                            let new_str = path.trim_start_matches(route);
                            path.len() - new_str.len()
                        });

                    match best {
                        Some((route, element))
                            if matched_route.is_none_or(|matched| &matched.0 != route) =>
                        {
                            let new_path = path.trim_start_matches(route);
                            let inserter = element.clone();

                            let route = route.clone();
                            commands.queue(move |world: &mut World| -> Result {
                                let components = world.components();
                                let route_id = components.component_id::<Route>().unwrap();
                                let child_id = components.component_id::<ChildOf>().unwrap();

                                let mut entity = world.get_entity_mut(child_entity)?;
                                let archetype = entity.archetype();
                                let components: Vec<_> = archetype
                                    .components()
                                    .filter(|c| *c != route_id && *c != child_id)
                                    .collect();

                                entity.despawn_related::<Children>();
                                entity.remove_by_ids(&components);
                                (inserter.0)(&mut entity);
                                entity.insert(MatchedRoute(route));

                                Ok(())
                            });

                            temp_path = new_path.into()
                        }
                        Some((route, ..)) => {
                            let new_path = path.trim_start_matches(route);
                            temp_path = new_path.into()
                        }
                        _ => {
                            temp_path = path.clone();
                        }
                    }

                    &mut temp_path
                }
                None => path,
            };

            find_routes(nodes, child_entity, path, commands)?;
        }

        Ok(())
    }

    let mut path = pathname.pathname.clone();

    // // only run when the pathname changes
    // if pathname.previous_path.as_deref() != Some(pathname.pathname.as_ref()) {
    let body = body.single()?;
    find_routes(&nodes, body, &mut path, &mut commands)?;
    // }

    Ok(())
}

#[macro_export]
macro_rules! routes {
    ($($route:literal => $element:expr),*$(,)?) => {
        {
            let mut route = $crate::router::Route::default();
            route
                $(.push($route, move || $element))*;
            route
        }
    };
}
