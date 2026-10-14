use html5ever::{
    parse_document,
    serialize::{serialize, SerializeOpts},
    tendril::TendrilSink,
    ParseOpts,
};
use markup5ever_rcdom::{NodeData, RcDom, SerializableHandle};

/// Compare parsed documents, not entity spellings or attribute order. This is a
/// diagnostics-only projection; it never rewrites stored or displayed messages.
pub fn canonical(html: &str) -> String {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let mut pending = vec![dom.document.clone()];
    while let Some(node) = pending.pop() {
        if let NodeData::Element { attrs, .. } = &node.data {
            attrs.borrow_mut().sort_by(|a, b| {
                a.name
                    .ns
                    .cmp(&b.name.ns)
                    .then(a.name.local.cmp(&b.name.local))
            });
        }
        pending.extend(node.children.borrow().iter().cloned());
    }
    let mut buf = Vec::new();
    serialize(
        &mut buf,
        &SerializableHandle::from(dom.document),
        SerializeOpts::default(),
    )
    .expect("DOM serialization");
    String::from_utf8(buf).expect("UTF-8 DOM serialization")
}
