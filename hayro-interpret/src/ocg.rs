use hayro_syntax::object::dict::keys::{
    BASE_STATE, D, OCG, OCGS, OCMD, OCPROPERTIES, OFF, ON, P, TYPE,
};
use hayro_syntax::object::{Array, Dict, Name, ObjectIdentifier};
use std::collections::HashSet;

pub(crate) struct OcgState {
    inactive_ocgs: HashSet<ObjectIdentifier>,
    content_stack: Vec<bool>,
}

impl OcgState {
    fn dummy() -> Self {
        Self {
            inactive_ocgs: HashSet::default(),
            content_stack: Vec::new(),
        }
    }

    pub(crate) fn from_catalog(catalog: &Dict<'_>) -> Self {
        let Some(oc_properties) = catalog.get::<Dict<'_>>(OCPROPERTIES) else {
            return Self::dummy();
        };

        let Some(config) = oc_properties.get::<Dict<'_>>(D) else {
            return Self::dummy();
        };

        let mut inactive = HashSet::new();

        let base_state = config
            .get::<Name<'_>>(BASE_STATE)
            .and_then(|b| BaseState::from_name(b.as_ref()));

        if base_state.unwrap_or(BaseState::On) == BaseState::Off
            && let Some(ocgs) = oc_properties.get::<Array<'_>>(OCGS)
        {
            for item in ocgs.raw_iter() {
                if let Some(ref_) = item.as_obj_ref() {
                    let id: ObjectIdentifier = ref_.into();
                    inactive.insert(id);
                }
            }
        }

        let mut read_ocg_array = |key, insert_active: bool| {
            if let Some(arr) = config.get::<Array<'_>>(key) {
                for item in arr.raw_iter() {
                    if let Some(ref_) = item.as_obj_ref() {
                        let id: ObjectIdentifier = ref_.into();
                        if insert_active {
                            inactive.remove(&id);
                        } else {
                            inactive.insert(id);
                        }
                    }
                }
            }
        };

        read_ocg_array(ON, true);
        read_ocg_array(OFF, false);

        Self {
            inactive_ocgs: inactive,
            content_stack: Vec::new(),
        }
    }

    fn ocmd_is_visible(&self, ocmd: &Dict<'_>) -> bool {
        let policy = ocmd
            .get::<Name<'_>>(P)
            .and_then(|n| OcmdPolicy::from_name(n.as_ref()))
            .unwrap_or(OcmdPolicy::AnyOn);

        let mut ocg_ids: Vec<ObjectIdentifier> = Vec::new();

        if let Some(arr) = ocmd.get::<Array<'_>>(OCGS) {
            for item in arr.raw_iter() {
                if let Some(ref_) = item.as_obj_ref() {
                    ocg_ids.push(ref_.into());
                }
            }
        } else if let Some(ref_) = ocmd.get_ref(OCGS) {
            ocg_ids.push(ref_.into());
        }

        if ocg_ids.is_empty() {
            true
        } else {
            match policy {
                OcmdPolicy::AllOn => ocg_ids.iter().all(|id| !self.inactive_ocgs.contains(id)),
                OcmdPolicy::AnyOn => ocg_ids.iter().any(|id| !self.inactive_ocgs.contains(id)),
                OcmdPolicy::AnyOff => ocg_ids.iter().any(|id| self.inactive_ocgs.contains(id)),
                OcmdPolicy::AllOff => ocg_ids.iter().all(|id| self.inactive_ocgs.contains(id)),
            }
        }
    }

    pub(crate) fn begin_optional_content(
        &mut self,
        props: &Dict<'_>,
        id: Option<ObjectIdentifier>,
    ) -> bool {
        let visible = match props.get::<Name<'_>>(TYPE).as_deref() {
            Some(OCMD) => self.ocmd_is_visible(props),
            Some(OCG) => id.is_none_or(|id| !self.inactive_ocgs.contains(&id)),
            _ => return false,
        };

        self.content_stack.push(self.is_visible() && visible);

        true
    }

    pub(crate) fn begin_marked_content(&mut self) {
        self.content_stack.push(self.is_visible());
    }

    pub(crate) fn end_marked_content(&mut self) {
        self.content_stack.pop();
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.content_stack.last().copied().unwrap_or(true)
    }

    pub(crate) fn depth(&self) -> usize {
        self.content_stack.len()
    }
}

impl Default for OcgState {
    fn default() -> Self {
        Self::dummy()
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum BaseState {
    On,
    Off,
    Unchanged,
}

impl BaseState {
    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"ON" => Some(Self::On),
            b"OFF" => Some(Self::Off),
            b"Unchanged" => Some(Self::Unchanged),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum OcmdPolicy {
    AllOn,
    AnyOn,
    AnyOff,
    AllOff,
}

impl OcmdPolicy {
    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"AllOn" => Some(Self::AllOn),
            b"AnyOn" => Some(Self::AnyOn),
            b"AnyOff" => Some(Self::AnyOff),
            b"AllOff" => Some(Self::AllOff),
            _ => None,
        }
    }
}
