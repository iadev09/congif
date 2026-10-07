use std::collections::BTreeMap;

use crate::{BoxError, ConfigParser, ConfigSource, ConfigValue, async_trait};

/// The document element is a wrapper. Child elements become fields, attributes
/// use `@name`, and text on structured elements uses `$text`. Repeated child
/// names become sequences. Leaf text remains a string without type inference.
/// Namespaces and mixed content are rejected rather than silently flattened.
/// DTD processing is disabled by the XML parser's default options.
pub struct XmlParser;

#[async_trait]
impl ConfigParser for XmlParser {
    fn extensions(&self) -> &[&str] {
        &["xml"]
    }

    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        let document =
            roxmltree::Document::parse(std::str::from_utf8(source.bytes)?)?;
        element(document.root_element())
    }
}

fn element(node: roxmltree::Node<'_, '_>) -> Result<ConfigValue, BoxError> {
    if node.tag_name().namespace().is_some()
        || node.attributes().any(|a| a.namespace().is_some())
    {
        return Err(std::io::Error::other(
            "XML config namespaces are not supported",
        )
        .into());
    }
    let mut fields = BTreeMap::new();
    for attribute in node.attributes() {
        fields.insert(
            ConfigValue::String(format!("@{}", attribute.name())),
            ConfigValue::String(attribute.value().into()),
        );
    }
    let mut children: BTreeMap<String, Vec<ConfigValue>> = BTreeMap::new();
    for child in node.children().filter(|child| child.is_element()) {
        children
            .entry(child.tag_name().name().into())
            .or_default()
            .push(element(child)?);
    }
    let text: String = node
        .children()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect();
    if !children.is_empty() && !text.trim().is_empty() {
        return Err(std::io::Error::other(
            "XML config mixed content is not supported",
        )
        .into());
    }
    if children.is_empty() && fields.is_empty() {
        return Ok(ConfigValue::String(text));
    }
    if children.is_empty() && !text.is_empty() {
        fields.insert(
            ConfigValue::String("$text".into()),
            ConfigValue::String(text),
        );
    }
    for (name, mut values) in children {
        let value = if values.len() == 1 {
            values.remove(0)
        } else {
            ConfigValue::Seq(values)
        };
        fields.insert(ConfigValue::String(name), value);
    }
    Ok(ConfigValue::Map(fields))
}
