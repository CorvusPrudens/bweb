use crate::dom::{DomStartupSystems, prelude::*};
use crate::dom::{DomSystems, events::Events};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_ecs::system::{SystemId, SystemParam};
use bevy_platform::collections::HashMap;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsValue;

pub mod query;

// TODO: okay this should probably be a lil entity set guy
#[derive(Default)]
#[cfg_attr(feature = "debug", derive(Debug))]
pub struct RouterPlugin;

impl Plugin for RouterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(query::QueryPlugin)
            .add_systems(
                PreStartup,
                initialize_router.in_set(DomStartupSystems::Pathname),
            )
            .add_systems(
                PostUpdate,
                (
                    resolve_routes
                        .in_set(DomSystems::ResolveRoutes)
                        .run_if(resource_changed::<Pathname>),
                    hook_into_anchors
                        .after(DomSystems::Reparent)
                        .before(DomSystems::Attach),
                ),
            )
            .init_resource::<RouteParams>()
            .init_resource::<NavigationGuard>()
            .add_observer(on_proceed)
            .add_observer(on_cancel);

        #[cfg(all(debug_assertions, feature = "debug"))]
        app.add_systems(
            PostUpdate,
            (|pathname: Res<Pathname>| {
                log::info!("navigation: {pathname:?}");
            })
            .run_if(resource_changed::<Pathname>),
        );
    }
}

#[derive(Resource)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
#[cfg_attr(feature = "reflect", reflect(Resource))]
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

fn initialize_router(
    window: Single<(Entity, &Window)>,
    mut guard: ResMut<NavigationGuard>,
    mut commands: Commands,
) -> Result {
    let (window_entity, window) = window.into_inner();

    commands.spawn((
        EventOf(window_entity),
        ev::pop_state(
            |_: Ev<web_sys::PopStateEvent>,
             window: Single<&Window>,
             mut commands: Commands|
             -> Result {
                let base = window.location().href().js_err()?;
                let url = web_sys::Url::new(&base).js_err()?;
                let new_href = url.href();
                let new_path = url.pathname();

                // The browser has already moved history; defer the decision to
                // the guard, which either commits or parks it and re-pushes our
                // prior location so the address bar stays honest.
                commands.queue(move |world: &mut World| resolve_pop(world, new_href, new_path));

                Ok(())
            },
        ),
    ));

    let location = window.location();
    let base = location.href().js_err()?;
    let url = web_sys::Url::new(&base).js_err()?;
    let pathname = url.pathname();

    guard.current_href = base;

    commands.insert_resource(Pathname {
        pathname,
        previous_path: None,
    });
    commands.insert_resource(query::QueryParams::from_url(&url));

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
                move |ev: Ev<web_sys::PointerEvent>, mut commands: Commands| {
                    if ev.ctrl_key() || ev.meta_key() {
                        return;
                    }

                    ev.prevent_default();
                    request_push(&mut commands, href.clone(), path.clone());
                },
            ),
        ));
    }

    Ok(())
}

#[derive(SystemParam)]
pub struct Navigator<'w, 's> {
    window: Single<'w, 's, &'static Window>,
    commands: Commands<'w, 's>,
}

#[cfg(feature = "debug")]
impl std::fmt::Debug for Navigator<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Navigator").finish_non_exhaustive()
    }
}

impl<'w, 's> Navigator<'w, 's> {
    pub fn navigate(&mut self, href: &str) -> Result<()> {
        // TODO: consider doing nothing when the urls are identical
        let base = self.window.location().href().js_err()?;

        let absolute = web_sys::Url::new(href).is_ok();
        if absolute {
            // no need to intercept
            return Ok(());
        }

        let url = web_sys::Url::new_with_base(href, &base).js_err()?;
        // Route through the guard rather than committing directly, so unsaved
        // work can veto or defer the navigation.
        request_push(&mut self.commands, url.href(), url.pathname());

        Ok(())
    }
}

/// Central navigation choke point. Every navigation resolves through here:
/// app-registered blocker predicates (see [`NavigationGuardExt`]) decide whether
/// it commits immediately or is parked as [`Self::pending`] until the app
/// triggers [`NavigationProceed`] or [`NavigationCancel`].
#[derive(Resource, Default)]
pub struct NavigationGuard {
    blockers: Vec<SystemId<(), bool>>,
    pending: Option<NavigationIntent>,
    /// The full href the user is currently on. Tracked here because on
    /// `popstate` the browser has already discarded the prior location and
    /// [`Pathname`] only retains a bare pathname (no host/query).
    current_href: String,
}

/// A navigation intercepted and parked awaiting an app-level decision.
#[derive(Clone)]
#[cfg_attr(feature = "debug", derive(Debug))]
struct NavigationIntent {
    href: String,
    path: String,
}

/// Fired when a registered blocker vetoes a navigation. The app resolves it by
/// triggering [`NavigationProceed`] (commit) or [`NavigationCancel`] (stay).
#[derive(Event, Clone)]
#[cfg_attr(feature = "debug", derive(Debug))]
pub struct NavigationBlocked {
    pub href: String,
    pub path: String,
}

/// Commit the parked navigation.
#[derive(Event)]
pub struct NavigationProceed;

/// Drop the parked navigation.
#[derive(Event)]
pub struct NavigationCancel;

/// Register a predicate that can veto navigations. If any registered blocker
/// returns `true`, the navigation is parked and [`NavigationBlocked`] is fired
/// instead of committing.
pub trait NavigationGuardExt {
    fn add_navigation_blocker<M>(
        &mut self,
        blocker: impl IntoSystem<(), bool, M> + 'static,
    ) -> &mut Self;
}

impl NavigationGuardExt for App {
    fn add_navigation_blocker<M>(
        &mut self,
        blocker: impl IntoSystem<(), bool, M> + 'static,
    ) -> &mut Self {
        self.init_resource::<NavigationGuard>();
        let id = self.world_mut().register_system(blocker);
        self.world_mut()
            .resource_mut::<NavigationGuard>()
            .blockers
            .push(id);
        self
    }
}

/// Run every registered blocker; `true` if any vetoes the navigation.
fn blocked(world: &mut World) -> bool {
    let ids = world.resource::<NavigationGuard>().blockers.clone();
    ids.into_iter()
        .any(|id| world.run_system(id).unwrap_or(false))
}

/// Push a new history entry for `href` (used by anchor/`navigate` commits and
/// when proceeding a parked intent).
fn push_history(href: &str) {
    if let Some(history) = web_sys::window().and_then(|w| w.history().ok()) {
        let _ = history.push_state_with_url(&JsValue::NULL, "", Some(href));
    }
}

/// Update the in-app router state (params + pathname + tracked href). Does not
/// touch browser history.
fn commit_state(world: &mut World, href: &str, path: &str) {
    if let Ok(url) = web_sys::Url::new(href) {
        world.resource_mut::<query::QueryParams>().update(&url);
    }
    world.resource_mut::<Pathname>().update(path.to_string());
    world.resource_mut::<NavigationGuard>().current_href = href.to_string();
}

/// Commit a push-style navigation (anchor / `navigate`, and proceeding a parked
/// intent): a new history entry plus the router state update.
fn commit_push(world: &mut World, href: &str, path: &str) {
    push_history(href);
    commit_state(world, href, path);
}

/// Commit a clean back/forward: the browser already moved history, so only the
/// router state is updated.
fn commit_soft(world: &mut World, href: &str, path: &str) {
    commit_state(world, href, path);
}

/// Entry point for push-style navigations (anchor click, `Navigator::navigate`).
/// Commits immediately when unblocked; otherwise parks the intent and fires
/// [`NavigationBlocked`].
fn request_push(commands: &mut Commands, href: String, path: String) {
    commands.queue(move |world: &mut World| {
        if world.resource::<NavigationGuard>().pending.is_some() {
            // A decision is already in flight; coalesce.
            return;
        }
        if !blocked(world) {
            commit_push(world, &href, &path);
            return;
        }
        world.resource_mut::<NavigationGuard>().pending = Some(NavigationIntent {
            href: href.clone(),
            path: path.clone(),
        });
        world.trigger(NavigationBlocked { href, path });
    });
}

/// Entry point for back/forward navigations. The browser has already moved, so
/// when blocked we re-push the prior location to keep the address bar honest
/// until the app decides.
fn resolve_pop(world: &mut World, new_href: String, new_path: String) {
    let prior = world.resource::<NavigationGuard>().current_href.clone();

    if world.resource::<NavigationGuard>().pending.is_some() {
        // User pressed Back again while a decision is pending; keep them put.
        push_history(&prior);
        return;
    }
    if !blocked(world) {
        commit_soft(world, &new_href, &new_path);
        return;
    }

    push_history(&prior);
    world.resource_mut::<NavigationGuard>().pending = Some(NavigationIntent {
        href: new_href.clone(),
        path: new_path.clone(),
    });
    world.trigger(NavigationBlocked {
        href: new_href,
        path: new_path,
    });
}

/// Commit the parked navigation. Always a push: a push intent never touched
/// history, and a `Pop` intent's prior was re-pushed at block time, so the
/// target must now be pushed. (A confirmed back thus becomes a forward push,
/// with the caveat that the history stack gains a duplicate entry.)
fn on_proceed(
    _: On<NavigationProceed>,
    mut guard: ResMut<NavigationGuard>,
    mut commands: Commands,
) {
    if let Some(intent) = guard.pending.take() {
        commands.queue(move |world: &mut World| {
            commit_push(world, &intent.href, &intent.path);
        });
    }
}

fn on_cancel(_: On<NavigationCancel>, mut guard: ResMut<NavigationGuard>) {
    guard.pending = None;
}

#[derive(Clone)]
struct RouteElement(Arc<Mutex<dyn FnMut(&mut World, Entity) + Send + Sync>>);

#[derive(Component, Default)]
pub struct Route {
    routes: Vec<(RouterPath, RouteElement)>,
}

#[cfg(feature = "debug")]
impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field(
                "routes",
                &self.routes.iter().map(|(p, _)| p).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
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

#[derive(Resource, Default)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
#[cfg_attr(feature = "reflect", reflect(Resource))]
pub struct RouteParams(HashMap<String, String>);

impl RouteParams {
    pub fn get(&self, param: &str) -> Option<&str> {
        self.0.get(param).map(|s| s.as_str())
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

#[derive(PartialEq, Eq)]
#[cfg_attr(any(test, feature = "debug"), derive(Debug))]
#[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
pub enum PathSegment {
    Root,
    Static(Cow<'static, str>),
    Param(Cow<'static, str>),
}

#[derive(Debug)]
pub struct PathSegmentError;

impl PathSegment {
    fn from_static(segment: &'static str) -> Self {
        match segment.strip_prefix(':') {
            Some(param) => Self::Param(Cow::Borrowed(param)),
            None => Self::Static(Cow::Borrowed(segment)),
        }
    }
}

#[derive(PartialEq, Eq)]
#[cfg_attr(any(test, feature = "debug"), derive(Debug))]
#[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
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

#[derive(PartialEq)]
#[cfg_attr(any(test, feature = "debug"), derive(Debug))]
struct RouteParseResult<'a> {
    matched: &'a str,
    remainder: &'a str,
    params: HashMap<String, String>,
}

#[derive(Component)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
#[cfg_attr(feature = "reflect", reflect(Component))]
pub struct MatchedRoute(String);

fn resolve_routes(
    body: Query<Entity, With<Body>>,
    nodes: Query<(Option<&Route>, Option<&MatchedRoute>, Option<&Children>)>,
    pathname: Res<Pathname>,
    mut route_params: ResMut<RouteParams>,
    mut commands: Commands,
) -> Result {
    fn find_routes(
        nodes: &Query<(Option<&Route>, Option<&MatchedRoute>, Option<&Children>)>,
        parent_entity: Entity,
        path: &mut &str,
        route_params: &mut RouteParams,
        commands: &mut Commands,
    ) -> Result {
        let (.., children) = nodes.get(parent_entity)?;

        for child_entity in children.iter().flat_map(|c| c.iter()) {
            let Ok((child_route, matched_route, ..)) = nodes.get(child_entity) else {
                continue;
            };

            if let Some(route) = child_route {
                let mut routes = route.routes.iter().collect::<Vec<_>>();
                routes.sort_unstable_by(|a, b| a.0.cmp_specificity(&b.0).reverse());

                let mut best = routes.into_iter().filter_map(|(route, el)| {
                    let parse_result = route.parse_path(path).ok()?;
                    Some((el, parse_result))
                });

                match best.next() {
                        Some((element, parse_result))
                            // TODO: this isn't quite right
                            if matched_route
                                .is_none_or(|matched| matched.0 != parse_result.matched) =>
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

            if char == '/' && self.start < i {
                let segment = &self.string[self.start + 1..i];
                self.start = i;

                return Some(segment);
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

        let input = "/static/coolio/last";

        let params = result.parse_path(input).unwrap();

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

        let input = "/static";

        let result = result.parse_path(input);

        assert!(result.is_err());
    }
}
