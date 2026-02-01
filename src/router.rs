use crate::dom::{DomStartupSystems, prelude::*};
use crate::dom::{DomSystems, events::Events};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use bevy_platform::collections::HashMap;
use log::info;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::sync::{Arc, Mutex};
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
        )
        .init_resource::<RouteParams>();

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
        ev::pop_state(
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
            &attr::Href,
            Option<&Events>,
            Has<attr::Download>,
            Option<&attr::Target>,
        ),
        (With<A>, Changed<attr::Href>),
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
            ev::click(
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

#[derive(SystemParam)]
pub struct Navigator<'w, 's> {
    window: Single<'w, 's, &'static Window>,
    pathname: ResMut<'w, Pathname>,
}

impl<'w, 's> Navigator<'w, 's> {
    pub fn navigate(&mut self, href: &str) -> Result<()> {
        let location = self.window.location();
        let base = location.href().js_err()?;

        let absolute = web_sys::Url::new(href).is_ok();
        if absolute {
            // no need to intercept
            return Ok(());
        }

        let url = web_sys::Url::new_with_base(href, &base).js_err()?;
        let href = url.href();
        let path = url.pathname();

        self.window
            .history()
            .unwrap()
            .push_state_with_url(&JsValue::NULL, "", Some(&href))
            .unwrap();
        self.pathname.update(path.clone());

        Ok(())
    }
}

#[derive(Clone)]
struct RouteElement(Arc<Mutex<dyn FnMut(&mut World, Entity) + Send + Sync>>);

#[derive(Component, Default)]
pub struct Route {
    routes: Vec<(RouterPath, RouteElement)>,
}

impl Route {
    pub const fn new() -> Self {
        Self { routes: Vec::new() }
    }

    pub fn route<F, B, M>(mut self, route: &'static str, element: F) -> Self
    where
        F: IntoSystem<(), B, M>,
        F::System: Send + Sync + 'static,
        // F: Fn() -> B + Send + Sync + 'static,
        B: Bundle,
    {
        let mut system = IntoSystem::into_system(element);

        let element = RouteElement(Arc::new(Mutex::new(
            move |world: &mut World, entity: Entity| {
                system.initialize(world);
                let bundle = system.run((), world).unwrap();
                world.entity_mut(entity).insert(bundle);
            },
        )));

        let route = RouterPath::from_static(route).expect("route string should be well-formed");

        self.routes.push((route, element));
        self
    }
}

#[derive(Resource, Default, Debug)]
pub struct RouteParams(HashMap<String, String>);

impl RouteParams {
    pub fn get(&self, param: &str) -> Option<&str> {
        self.0.get(param).map(|s| s.as_str())
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PathSegment {
    Root,
    Static(Cow<'static, str>),
    Param(Cow<'static, str>),
}

#[derive(Debug)]
pub struct PathSegmentError;

impl PathSegment {
    fn from_static(segment: &'static str) -> Self {
        if segment.starts_with(':') {
            Self::Param(Cow::Borrowed(&segment[1..]))
        } else {
            Self::Static(Cow::Borrowed(segment))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RouterPath(Vec<PathSegment>);

impl RouterPath {
    pub fn from_static(path: &'static str) -> Result<Self, PathSegmentError> {
        if path.is_empty() {
            return Err(PathSegmentError);
        }

        if path == "/" {
            return Ok(Self(vec![PathSegment::Root]));
        }

        Ok(Self(
            path.split('/')
                .filter(|segment| !segment.is_empty())
                .map(PathSegment::from_static)
                .collect(),
        ))
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn is_root(&self) -> bool {
        self.0.len() == 1 && self.0[0] == PathSegment::Root
    }

    fn parse_path<'a>(&self, url: &'a str) -> Result<RouteParseResult<'a>> {
        if url == "/" && self.is_root() {
            let result = RouteParseResult {
                matched: &url[..1],
                remainder: &url[1..],
                params: Default::default(),
            };

            return Ok(result);
        }

        let mut params = HashMap::new();

        let mut tracked_split = TrackedSplit::new(url);

        for segment in self.0.iter() {
            let Some(input) = tracked_split.next() else {
                return Err("unexpected path end".into());
            };

            match segment {
                PathSegment::Root => return Err("unexpected root".into()),
                PathSegment::Static(segment) => {
                    if segment != input {
                        return Err("failed to match static path segment".into());
                    }
                }
                PathSegment::Param(param_name) => {
                    params.insert(param_name.to_string(), input.to_string());
                }
            }
        }

        Ok(RouteParseResult {
            matched: tracked_split.consumed(),
            remainder: tracked_split.remainder(),
            params,
        })
    }

    /// Compare the specificity of two path patterns.
    fn cmp_specificity(&self, other: &Self) -> Ordering {
        match self.len().cmp(&other.len()) {
            Ordering::Equal => {}
            other => return other,
        }

        for pair in self.0.iter().zip(&other.0) {
            match pair {
                (PathSegment::Static(_), PathSegment::Param(_)) => {
                    return Ordering::Greater;
                }
                (PathSegment::Param(_), PathSegment::Static(_)) => {
                    return Ordering::Less;
                }
                _ => {}
            }
        }

        Ordering::Equal
    }

    // pub fn compare(&self, other: &Self, path: &str) -> core::cmp::Ordering {
    //     if path == "/" {
    //         match (self.is_root(), other.is_root())

    //         if self.is_root() {
    //             return core::cmp::Ordering::
    //         }
    //     }
    // }
}

#[derive(Debug, PartialEq)]
struct RouteParseResult<'a> {
    matched: &'a str,
    remainder: &'a str,
    params: HashMap<String, String>,
}

#[derive(Component)]
pub struct MatchedRoute(String);

fn resolve_routes(
    body: Query<Entity, With<Body>>,
    nodes: Query<(Option<&Route>, Option<&MatchedRoute>, Option<&Children>)>,
    pathname: Res<Pathname>,
    mut route_params: ResMut<RouteParams>,
    mut commands: Commands,
) -> Result {
    fn find_routes<'a>(
        nodes: &Query<(Option<&Route>, Option<&MatchedRoute>, Option<&Children>)>,
        parent_entity: Entity,
        path: &mut &'a str,
        route_params: &mut RouteParams,
        commands: &mut Commands,
    ) -> Result {
        let (.., children) = nodes.get(parent_entity)?;

        for child_entity in children.iter().flat_map(|c| c.iter()) {
            let Ok((child_route, matched_route, ..)) = nodes.get(child_entity) else {
                continue;
            };

            match child_route {
                Some(route) => {
                    let mut routes = route.routes.iter().collect::<Vec<_>>();
                    routes.sort_unstable_by(|a, b| a.0.cmp_specificity(&b.0).reverse());

                    let mut best = routes.into_iter().filter_map(|(route, el)| {
                        let parse_result = route.parse_path(*path).ok()?;
                        Some((el, parse_result))
                    });

                    match best.next() {
                        Some((element, parse_result))
                            // TODO: this isn't quite right
                            if matched_route
                                .is_none_or(|matched| &matched.0 != parse_result.matched) =>
                        {
                            let inserter = element.clone();

                            let RouteParseResult { matched, remainder, params } = parse_result;
                            let matched = matched.to_string();

                            route_params.0.extend(params);

                            commands.queue(move |world: &mut World| -> Result {
                                let components = world.components();
                                let route_id = components.component_id::<Route>().unwrap();
                                let child_id = components.component_id::<ChildOf>().unwrap();

                                let mut entity = world.get_entity_mut(child_entity)?;
                                let archetype = entity.archetype();
                                let components: Vec<_> = archetype
                                    .components()
                                    .iter()
                                    .copied()
                                    .filter(|c| *c != route_id && *c != child_id)
                                    .collect();

                                entity.despawn_related::<Children>();
                                entity.remove_by_ids(&components);

                                (inserter.0.lock().unwrap())(world, child_entity);

                                world
                                    .entity_mut(child_entity)
                                    .insert(MatchedRoute(matched));

                                Ok(())
                            });

                            *path = remainder;
                        }
                        Some((_, parse_result)) => {
                            *path = parse_result.remainder;
                        }
                        _ => {}
                    }
                }
                None => {}
            };

            find_routes(nodes, child_entity, path, route_params, commands)?;
        }

        Ok(())
    }

    let path = pathname.pathname.clone();
    let path = &mut path.as_str();

    // // only run when the pathname changes
    // if pathname.previous_path.as_deref() != Some(pathname.pathname.as_ref()) {
    route_params.clear();
    let body = body.single()?;
    find_routes(&nodes, body, path, &mut route_params, &mut commands)?;
    // }

    Ok(())
}

struct TrackedSplit<'a> {
    string: &'a str,
    start: usize,
}

impl<'a> TrackedSplit<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            string: input,
            start: 0,
        }
    }

    pub fn consumed(&self) -> &'a str {
        if self.start == 0 {
            ""
        } else {
            &self.string[..self.start]
        }
    }

    pub fn remainder(&self) -> &'a str {
        if self.start == self.string.len() {
            ""
        } else {
            &self.string[self.start..]
        }
    }
}

impl<'a> Iterator for TrackedSplit<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start == self.string.len() {
            return None;
        }

        for (i, char) in self.string[self.start..].char_indices() {
            let i = i + self.start;

            if char == '/' {
                if self.start < i {
                    let segment = &self.string[self.start + 1..i];
                    self.start = i;

                    return Some(segment);
                }
            }
        }

        let segment = &self.string[self.start + 1..];
        self.start = self.string.len();

        Some(segment)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_root() {
        let input = "/";
        let result = RouterPath::from_static(input).unwrap();

        assert_eq!(result, RouterPath(vec![PathSegment::Root,]));
    }

    #[test]
    fn test_basic_url() {
        let input = "/static/:param";
        let result = RouterPath::from_static(input).unwrap();

        assert_eq!(
            result,
            RouterPath(vec![
                PathSegment::Static("static".into()),
                PathSegment::Param("param".into())
            ])
        );
    }

    #[test]
    fn test_parse() {
        let pattern = "/static/:param";
        let result = RouterPath::from_static(pattern).unwrap();

        let mut input = "/static/coolio/last";

        let params = result.parse_path(&mut input).unwrap();

        assert_eq!(
            params,
            RouteParseResult {
                matched: "/static/coolio",
                remainder: "/last",
                params: HashMap::from_iter([(String::from("param"), String::from("coolio"))])
            }
        );
    }

    #[test]
    fn test_input_too_short() {
        let pattern = "/static/:param";
        let result = RouterPath::from_static(pattern).unwrap();

        let mut input = "/static";

        let result = result.parse_path(&mut input);

        assert!(result.is_err());
    }
}
