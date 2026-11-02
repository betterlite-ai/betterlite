use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteId(u64);

#[derive(Debug, Clone)]
pub struct InstalledRoute {
    pub id: RouteId,
    pub path: String,
    pub methods: BTreeSet<String>,
    pub lazy_owned: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RouteTable {
    routes: Vec<InstalledRoute>,
    next_id: u64,
}

impl RouteTable {
    pub fn install(
        &mut self,
        path: String,
        methods: BTreeSet<String>,
        lazy_owned: bool,
    ) -> RouteId {
        let id = RouteId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.routes.push(InstalledRoute {
            id,
            path,
            methods,
            lazy_owned,
        });
        id
    }

    #[must_use]
    pub fn colliding_routes(&self, path: &str, methods: &BTreeSet<String>) -> Vec<&InstalledRoute> {
        let mut matches = Vec::new();
        for route in &self.routes {
            if route.path == path && !route.methods.is_disjoint(methods) {
                matches.push(route);
            }
        }
        matches
    }

    #[must_use]
    pub fn is_path_registered(&self, path: &str, methods: &BTreeSet<String>) -> bool {
        for route in self.colliding_routes(path, methods) {
            if !route.lazy_owned {
                return true;
            }
        }
        false
    }

    // A collision with only lazy-owned routes still installs the new route, ahead of the first one.
    pub fn add_route_if_not_exists(&mut self, path: String, methods: BTreeSet<String>) -> bool {
        if self.is_path_registered(&path, &methods) {
            return false;
        }
        let before_index = self
            .colliding_routes(&path, &methods)
            .first()
            .map(|route| route.id)
            .and_then(|id| self.position_of(id));
        self.install(path, methods, false);
        if let Some(index) = before_index {
            self.reinsert_last(index);
        }
        true
    }

    #[must_use]
    pub fn order(&self) -> Vec<RouteId> {
        let mut ids = Vec::new();
        for route in &self.routes {
            ids.push(route.id);
        }
        ids
    }

    fn position_of(&self, id: RouteId) -> Option<usize> {
        for (index, route) in self.routes.iter().enumerate() {
            if route.id == id {
                return Some(index);
            }
        }
        None
    }

    fn reinsert_last(&mut self, index: usize) {
        if let Some(route) = self.routes.pop() {
            self.routes.insert(index, route);
        }
    }
}

mod tests;
