// SPDX-License-Identifier: Apache-2.0

//! Immutable views of resources, schema-less documents, and the project,
//! built once as frozen Starlark values. Repository code receives these; it
//! cannot mutate them, and nothing in them names the source they came from.

use serde_json::{Map, Value, json};
use starlark::environment::Module;
use starlark::values::dict::AllocDict;
use starlark::values::list::AllocList;
use starlark::values::{Heap, OwnedFrozenValue, Value as StarlarkValue};

use crate::document::Document;
use crate::envelope::Resource;
use crate::graph::Graph;

/// Frozen views for one project.
pub struct Views {
    /// One view per structurally valid resource, in resource order.
    pub resources: Vec<OwnedFrozenValue>,
    /// The project view.
    pub project: OwnedFrozenValue,
}

impl Views {
    /// Build the views over the structurally valid resources and the parsed
    /// documents. `indexes` are the resources' positions in the parsed
    /// resource list and in `graph`; `documents` are in path order.
    pub fn build(
        resources: &[Resource],
        indexes: &[usize],
        graph: &Graph,
        documents: &[Document],
    ) -> Result<Self, String> {
        let resource_json: Vec<Value> = indexes
            .iter()
            .map(|index| resource_json(*index, &resources[*index], graph))
            .collect();
        let document_json: Vec<Value> = documents.iter().map(Document::view).collect();
        let project_json = project_json(&resource_json, &document_json, graph);

        let frozen = Module::with_temp_heap(|module| {
            let heap = module.heap();
            for (index, value) in resource_json.iter().enumerate() {
                module.set(&format!("resource_{index}"), alloc_json(heap, value)?);
            }
            module.set("project", alloc_json(heap, &project_json)?);
            module.freeze().map_err(|error| format!("{error:?}"))
        })?;
        let mut views = Vec::with_capacity(indexes.len());
        for index in 0..indexes.len() {
            views.push(
                frozen
                    .get(&format!("resource_{index}"))
                    .map_err(|error| error.to_string())?,
            );
        }
        let project = frozen.get("project").map_err(|error| error.to_string())?;
        Ok(Self {
            resources: views,
            project,
        })
    }
}

/// Convert JSON to a Starlark value. A number that fits neither `i64` nor
/// `f64` is an error rather than a substituted value.
fn alloc_json<'v>(heap: Heap<'v>, value: &Value) -> Result<StarlarkValue<'v>, String> {
    Ok(match value {
        Value::Null => StarlarkValue::new_none(),
        Value::Bool(b) => StarlarkValue::new_bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                heap.alloc(i)
            } else if let Some(f) = n.as_f64() {
                heap.alloc(f)
            } else {
                return Err(format!(
                    "number {n} cannot be represented as an integer or a float"
                ));
            }
        }
        Value::String(s) => heap.alloc_str(s).to_value(),
        Value::Array(items) => {
            let items = items
                .iter()
                .map(|item| alloc_json(heap, item))
                .collect::<Result<Vec<_>, _>>()?;
            heap.alloc(AllocList(items))
        }
        Value::Object(map) => {
            let entries = map
                .iter()
                .map(|(key, item)| Ok((heap.alloc_str(key).to_value(), alloc_json(heap, item)?)))
                .collect::<Result<Vec<_>, String>>()?;
            heap.alloc(AllocDict(entries))
        }
    })
}

/// The JSON view of one resource.
pub fn resource_json(index: usize, resource: &Resource, graph: &Graph) -> Value {
    let fragments: Vec<Value> = resource
        .fragments
        .iter()
        .map(|fragment| {
            json!({
                "kind": fragment.kind,
                "id": fragment.id,
                "fields": fragment.fields,
                "line": fragment.line,
                "section": fragment.section,
            })
        })
        .collect();
    let referenced_by: Vec<Value> = graph.referenced_by[index]
        .iter()
        .map(|(from, field)| json!({ "from": from, "field": field }))
        .collect();
    json!({
        "id": resource.id,
        "schema": resource.schema,
        "path": resource.path.as_str(),
        "refs": resource.refs,
        "fields": resource.fields,
        "body": resource.body,
        "sections": resource.doc.sections,
        "anchors": resource.doc.anchors,
        "blocks": resource.doc.blocks,
        "links": resource.doc.links,
        "images": resource.doc.images,
        "fragments": fragments,
        "relations": graph.relations[index],
        "referenced_by": referenced_by,
    })
}

/// The JSON view of the whole project.
pub fn project_json(resources: &[Value], documents: &[Value], graph: &Graph) -> Value {
    let ids: Map<String, Value> = graph
        .nodes
        .iter()
        .map(|(id, node)| (id.clone(), Value::String(node.kind.clone())))
        .collect();
    let mut by_id = Map::new();
    let mut by_schema: Map<String, Value> = Map::new();
    for resource in resources {
        let id = resource["id"].as_str().unwrap_or_default().to_owned();
        let schema = resource["schema"].as_str().unwrap_or_default().to_owned();
        by_id.entry(id.clone()).or_insert_with(|| resource.clone());
        match by_schema
            .entry(schema)
            .or_insert_with(|| Value::Array(Vec::new()))
        {
            Value::Array(list) => list.push(Value::String(id)),
            _ => unreachable!("schema index holds arrays"),
        }
    }
    json!({
        "resources": resources,
        "by_id": by_id,
        "by_schema": by_schema,
        "ids": ids,
        "documents": documents,
    })
}
