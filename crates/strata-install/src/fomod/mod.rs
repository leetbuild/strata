//! FOMOD installers: `fomod/ModuleConfig.xml` and `fomod/info.xml`.
//!
//! The reading rules are MO2's `installer_fomod` (`FomodInstallerDialog::read*`, upstream
//! `modorganizer-installer_fomod` master, which Fluorine's `libs/installer_fomod` follows):
//! tags are matched case-sensitively, unknown elements are skipped, a `<file>` or `<folder>`
//! with an empty source is ignored, steps and plugins are sorted by name unless the order is
//! `Explicit` (the default order is `Ascending`), a group of one plugin that must be picked
//! becomes "select all". The install logic (steps, defaults, conditions) is in [`session`].

pub mod session;

use crate::{Error, Result};

/// `info.xml`: what the author says about the mod.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub name: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub id: Option<i64>,
    pub website: Option<String>,
}

/// `ModuleConfig.xml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleConfig {
    pub module_name: String,
    /// `moduleDependencies`: the install stops when they are not met.
    pub dependencies: Option<Composite>,
    pub required_files: Vec<FileInstall>,
    /// The steps in display order.
    pub steps: Vec<Step>,
    pub conditional_installs: Vec<ConditionalInstall>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub name: String,
    /// `visible`: the step is shown only when this holds.
    pub visible: Option<Composite>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    /// After MO2's correction of one-plugin groups.
    pub kind: GroupType,
    /// In display order.
    pub plugins: Vec<Plugin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum GroupType {
    SelectAtLeastOne,
    SelectAtMostOne,
    SelectExactlyOne,
    SelectAny,
    SelectAll,
}

impl GroupType {
    /// Radio buttons (one at most).
    pub fn is_radio(self) -> bool {
        matches!(self, GroupType::SelectAtMostOne | GroupType::SelectExactlyOne)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    pub name: String,
    pub description: String,
    /// Path of the picture, relative to the folder holding `fomod/`.
    pub image: Option<String>,
    /// Sorted by priority, then document order.
    pub files: Vec<FileInstall>,
    /// `conditionFlags` with a non-empty name: `(name, value)`.
    pub flags: Vec<(String, String)>,
    pub type_info: TypeInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum PluginType {
    Required,
    Optional,
    Recommended,
    NotUsable,
    CouldBeUsable,
}

/// The plugin's type: the first pattern whose condition holds, else the default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeInfo {
    pub default: PluginType,
    pub patterns: Vec<(Composite, PluginType)>,
}

/// `<file>` or `<folder>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInstall {
    /// `/`-separated, relative to the folder holding `fomod/`.
    pub source: String,
    /// `/` or `\` separated as written; for a file, a trailing separator means "into this
    /// folder"; the source when the attribute is missing.
    pub destination: String,
    pub is_folder: bool,
    pub priority: i64,
    /// Position in the document (`m_FileSystemItemSequence`), the tie-breaker of equal
    /// priorities.
    pub sequence: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalInstall {
    pub condition: Composite,
    pub files: Vec<FileInstall>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    And,
    Or,
}

/// `dependencies` (MO2's `SubCondition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composite {
    pub operator: Operator,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// `fileDependency`: `state` is `Missing`, `Inactive` or `Active`.
    File { file: String, state: String },
    /// `flagDependency`.
    Flag { name: String, value: String },
    /// `gameDependency`.
    Game { version: String },
    /// `fommDependency`.
    Fomm { version: String },
    /// `foseDependency` (the script extender).
    ScriptExtender { version: String },
    Nested(Composite),
}

/// Decodes an XML file the way MO2 ends up reading it: a BOM or the `<?` pattern says
/// UTF-16 or UTF-8; otherwise UTF-8 if valid, else Latin-1 (`FomodInstallerDialog::readXml`
/// tries UTF-16, UTF-8, Latin-1 after a failed strict parse). The declaration is dropped,
/// since its `encoding` is often wrong.
pub fn decode_xml(bytes: &[u8]) -> String {
    fn utf16(bytes: &[u8], le: bool) -> String {
        let units: Vec<u16> =
            bytes.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
        String::from_utf16_lossy(&units)
    }
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, true)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, false)
    } else if bytes.starts_with(&[0x3C, 0x00]) {
        utf16(bytes, true)
    } else if bytes.starts_with(&[0x00, 0x3C]) {
        utf16(bytes, false)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_owned(),
            Err(_) => bytes.iter().map(|&b| b as char).collect(),
        }
    };
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    match trimmed.strip_prefix("<?xml").and_then(|r| r.find("?>").map(|end| &r[end + 2..])) {
        Some(rest) => rest.to_owned(),
        None => trimmed.to_owned(),
    }
}

fn parse_doc<'a>(text: &'a str, file: &str) -> Result<roxmltree::Document<'a>> {
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    roxmltree::Document::parse_with_options(text, options)
        .map_err(|e| Error::Fomod(format!("{file} is not valid XML: {e}")))
}

type XNode<'a, 'i> = roxmltree::Node<'a, 'i>;

fn elements<'a, 'i>(node: XNode<'a, 'i>) -> impl Iterator<Item = XNode<'a, 'i>> {
    node.children().filter(|n| n.is_element())
}

fn tag<'a>(node: XNode<'a, '_>) -> &'a str {
    node.tag_name().name()
}

/// `XmlReader::getText`: every text piece of the element (comments skipped).
fn text(node: XNode) -> String {
    node.children().filter(|n| n.is_text()).filter_map(|n| n.text()).collect()
}

/// `readContent`: the element's first token when it is text.
fn content(node: XNode) -> String {
    node.first_child().filter(|n| n.is_text()).and_then(|n| n.text()).unwrap_or("").to_owned()
}

fn attr<'a>(node: XNode<'a, '_>, name: &str) -> Option<&'a str> {
    node.attribute(name)
}

impl Info {
    /// Reads `info.xml`. MO2 takes `Name`, `Author`, `Version`, `Id` and `Website` from
    /// anywhere in the document.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let xml = decode_xml(bytes);
        let doc = parse_doc(&xml, "info.xml")?;
        let mut info = Info::default();
        for node in doc.descendants().filter(|n| n.is_element()) {
            match tag(node) {
                "Name" => info.name = Some(content(node)),
                "Author" => info.author = Some(content(node)),
                "Version" => info.version = Some(content(node)),
                // QString::toInt: 0 when not a number
                "Id" => info.id = Some(content(node).trim().parse().unwrap_or(0)),
                "Website" => info.website = Some(content(node)),
                _ => {}
            }
        }
        Ok(info)
    }
}

/// `normalizeArchivePath`: `/` separators, `.` and `..` folded, no leading or trailing `/`.
pub fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

struct Reader {
    sequence: usize,
}

fn order_of(node: XNode) -> Result<&'static str> {
    match attr(node, "order").unwrap_or("Ascending") {
        "Ascending" => Ok("Ascending"),
        "Descending" => Ok("Descending"),
        "Explicit" => Ok("Explicit"),
        other => Err(Error::Fomod(format!("unsupported order type {other}"))),
    }
}

fn plugin_type(name: &str) -> PluginType {
    match name {
        "Required" => PluginType::Required,
        "Recommended" => PluginType::Recommended,
        "NotUsable" => PluginType::NotUsable,
        "CouldBeUsable" => PluginType::CouldBeUsable,
        // MO2: "invalid plugin type", treated as optional
        _ => PluginType::Optional,
    }
}

impl Reader {
    fn file_list(&mut self, node: XNode) -> Vec<FileInstall> {
        let mut files = Vec::new();
        for item in elements(node) {
            let is_folder = match tag(item) {
                "folder" => true,
                "file" => false,
                _ => continue,
            };
            let source = attr(item, "source").unwrap_or("");
            if source.is_empty() {
                continue;
            }
            let source = normalize_path(source);
            let destination = attr(item, "destination").map_or_else(|| source.clone(), str::to_owned);
            self.sequence += 1;
            files.push(FileInstall {
                source,
                destination,
                is_folder,
                priority: attr(item, "priority").and_then(|p| p.trim().parse().ok()).unwrap_or(0),
                sequence: self.sequence,
            });
        }
        files
    }

    fn composite(node: XNode) -> Composite {
        let operator = if attr(node, "operator") == Some("Or") { Operator::Or } else { Operator::And };
        let mut conditions = Vec::new();
        for item in elements(node) {
            let a = |name| attr(item, name).unwrap_or("").to_owned();
            conditions.push(match tag(item) {
                "fileDependency" => Condition::File { file: a("file"), state: a("state") },
                "flagDependency" => Condition::Flag { name: a("flag"), value: a("value") },
                "gameDependency" => Condition::Game { version: a("version") },
                "fommDependency" => Condition::Fomm { version: a("version") },
                "foseDependency" => Condition::ScriptExtender { version: a("version") },
                "dependencies" => Condition::Nested(Self::composite(item)),
                _ => continue,
            });
        }
        Composite { operator, conditions }
    }

    fn type_info(node: XNode) -> TypeInfo {
        let mut info = TypeInfo { default: PluginType::Optional, patterns: Vec::new() };
        for item in elements(node) {
            match tag(item) {
                "type" => info.default = plugin_type(attr(item, "name").unwrap_or("")),
                "dependencyType" => {
                    for part in elements(item) {
                        match tag(part) {
                            "defaultType" => info.default = plugin_type(attr(part, "name").unwrap_or("")),
                            "patterns" => {
                                for pattern in elements(part).filter(|p| tag(*p) == "pattern") {
                                    let mut condition = Composite { operator: Operator::And, conditions: Vec::new() };
                                    let mut kind = PluginType::Optional;
                                    for x in elements(pattern) {
                                        match tag(x) {
                                            "dependencies" => condition = Self::composite(x),
                                            "type" => kind = plugin_type(attr(x, "name").unwrap_or("")),
                                            _ => {}
                                        }
                                    }
                                    info.patterns.push((condition, kind));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        info
    }

    fn plugin(&mut self, node: XNode) -> Plugin {
        let mut plugin = Plugin {
            name: attr(node, "name").unwrap_or("").to_owned(),
            description: String::new(),
            image: None,
            files: Vec::new(),
            flags: Vec::new(),
            type_info: TypeInfo { default: PluginType::Optional, patterns: Vec::new() },
        };
        for item in elements(node) {
            match tag(item) {
                "description" => plugin.description = text(item).trim().to_owned(),
                "image" => plugin.image = attr(item, "path").map(str::to_owned),
                "files" => plugin.files = self.file_list(item),
                "conditionFlags" => {
                    for flag in elements(item).filter(|f| tag(*f) == "flag") {
                        let name = attr(flag, "name").unwrap_or("").to_owned();
                        if !name.is_empty() {
                            plugin.flags.push((name, text(flag).trim().to_owned()));
                        }
                    }
                }
                "typeDescriptor" => plugin.type_info = Self::type_info(item),
                _ => {}
            }
        }
        plugin.files.sort_by_key(|f| (f.priority, f.sequence));
        plugin
    }

    fn group(&mut self, node: XNode) -> Result<Group> {
        let name = attr(node, "name").unwrap_or("").to_owned();
        let mut kind = match attr(node, "type").unwrap_or("") {
            "SelectAtLeastOne" => GroupType::SelectAtLeastOne,
            "SelectAtMostOne" => GroupType::SelectAtMostOne,
            "SelectExactlyOne" => GroupType::SelectExactlyOne,
            "SelectAny" => GroupType::SelectAny,
            "SelectAll" => GroupType::SelectAll,
            other => return Err(Error::Fomod(format!("unsupported group type {other}"))),
        };
        let mut plugins = Vec::new();
        for list in elements(node).filter(|n| tag(*n) == "plugins") {
            let order = order_of(list)?;
            let mut these: Vec<Plugin> = elements(list).filter(|n| tag(*n) == "plugin").map(|p| self.plugin(p)).collect();
            // readPluginList: one plugin that must be picked is simply installed
            if these.len() == 1 {
                kind = match kind {
                    GroupType::SelectAtLeastOne | GroupType::SelectExactlyOne => GroupType::SelectAll,
                    GroupType::SelectAtMostOne => GroupType::SelectAny,
                    k => k,
                };
            }
            match order {
                "Ascending" => these.sort_by(|a, b| a.name.cmp(&b.name)),
                "Descending" => these.sort_by(|a, b| b.name.cmp(&a.name)),
                _ => {}
            }
            plugins.extend(these);
        }
        Ok(Group { name, kind, plugins })
    }

    fn step(&mut self, node: XNode) -> Result<Step> {
        let mut step = Step { name: attr(node, "name").unwrap_or("").to_owned(), visible: None, groups: Vec::new() };
        for item in elements(node) {
            match tag(item) {
                "visible" => {
                    let c = Self::composite(item);
                    step.visible = (!c.conditions.is_empty()).then_some(c);
                }
                "optionalFileGroups" => {
                    for g in elements(item).filter(|g| tag(*g) == "group") {
                        step.groups.push(self.group(g)?);
                    }
                }
                _ => {}
            }
        }
        Ok(step)
    }
}

impl ModuleConfig {
    /// Reads `ModuleConfig.xml`.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let xml = decode_xml(bytes);
        let doc = parse_doc(&xml, "ModuleConfig.xml")?;
        let root = doc.root_element();
        if tag(root) != "config" {
            return Err(Error::Fomod(format!("ModuleConfig.xml: expected <config>, got <{}>", tag(root))));
        }
        let mut reader = Reader { sequence: 0 };
        let mut config = ModuleConfig::default();
        for item in elements(root) {
            match tag(item) {
                "moduleName" => config.module_name = text(item),
                "moduleDependencies" => config.dependencies = Some(Reader::composite(item)),
                "requiredInstallFiles" => config.required_files = reader.file_list(item),
                "installSteps" => {
                    let order = order_of(item)?;
                    let mut steps = Vec::new();
                    for s in elements(item).filter(|s| tag(*s) == "installStep") {
                        steps.push(reader.step(s)?);
                    }
                    match order {
                        "Ascending" => steps.sort_by(|a, b| a.name.cmp(&b.name)),
                        "Descending" => steps.sort_by(|a, b| b.name.cmp(&a.name)),
                        _ => {}
                    }
                    config.steps = steps;
                }
                "conditionalFileInstalls" => {
                    for patterns in elements(item).filter(|p| tag(*p) == "patterns") {
                        for pattern in elements(patterns).filter(|p| tag(*p) == "pattern") {
                            let mut install = ConditionalInstall {
                                condition: Composite { operator: Operator::And, conditions: Vec::new() },
                                files: Vec::new(),
                            };
                            for x in elements(pattern) {
                                match tag(x) {
                                    "dependencies" => install.condition = Reader::composite(x),
                                    "files" => install.files = reader.file_list(x),
                                    _ => {}
                                }
                            }
                            config.conditional_installs.push(install);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<config xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <moduleName>Sample</moduleName>
  <requiredInstallFiles>
    <folder source="core" destination="" />
    <file source="" destination="ignored" />
  </requiredInstallFiles>
  <installSteps order="Explicit">
    <installStep name="Options">
      <optionalFileGroups order="Explicit">
        <group name="Textures" type="SelectExactlyOne">
          <plugins order="Explicit">
            <plugin name="2K">
              <description> Big </description>
              <image path="fomod\2k.png"/>
              <files><folder source="tex\2k" destination="textures" priority="1"/></files>
              <conditionFlags><flag name="res">2k</flag></conditionFlags>
              <typeDescriptor><type name="Recommended"/></typeDescriptor>
            </plugin>
            <plugin name="1K">
              <files><folder source="tex/1k" destination="textures"/></files>
              <conditionFlags><flag name="res">1k</flag></conditionFlags>
              <typeDescriptor><type name="Optional"/></typeDescriptor>
            </plugin>
          </plugins>
        </group>
        <group name="Lonely" type="SelectExactlyOne">
          <plugins><plugin name="Only"><typeDescriptor><type name="Optional"/></typeDescriptor></plugin></plugins>
        </group>
      </optionalFileGroups>
    </installStep>
    <installStep name="Patches">
      <visible><flagDependency flag="res" value="2k"/></visible>
      <optionalFileGroups>
        <group name="Patches" type="SelectAny">
          <plugins>
            <plugin name="Zeta"><typeDescriptor>
              <dependencyType><defaultType name="Optional"/>
                <patterns><pattern><dependencies operator="And"><fileDependency file="Zeta.esp" state="Active"/></dependencies><type name="Recommended"/></pattern></patterns>
              </dependencyType></typeDescriptor>
              <files><file source="patches/zeta.esp" destination="Zeta Patch.esp"/></files>
            </plugin>
            <plugin name="Alpha"><typeDescriptor><type name="NotUsable"/></typeDescriptor></plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
  </installSteps>
  <conditionalFileInstalls><patterns>
    <pattern><dependencies operator="Or"><flagDependency flag="res" value="1k"/><dependencies><gameDependency version="1.5.97"/></dependencies></dependencies>
      <files><file source="extra/readme.txt" destination="docs\" priority="-1"/></files></pattern>
  </patterns></conditionalFileInstalls>
</config>"#;

    #[test]
    fn module_config_reads_everything() {
        let c = ModuleConfig::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(c.module_name, "Sample");
        assert_eq!(c.required_files.len(), 1, "empty sources are ignored");
        assert_eq!(c.required_files[0].destination, "");
        assert_eq!(c.steps.len(), 2);
        let opts = &c.steps[0];
        assert_eq!(opts.groups[0].plugins[0].name, "2K", "explicit order kept");
        assert_eq!(opts.groups[0].plugins[0].description, "Big");
        assert_eq!(opts.groups[0].plugins[0].files[0].source, "tex/2k");
        assert_eq!(opts.groups[1].kind, GroupType::SelectAll, "one plugin to pick = select all");
        let patches = &c.steps[1].groups[0];
        assert_eq!(patches.plugins[0].name, "Alpha", "ascending by default");
        assert_eq!(patches.plugins[1].type_info.patterns.len(), 1);
        assert!(c.steps[1].visible.is_some());
        assert_eq!(c.conditional_installs[0].condition.operator, Operator::Or);
        assert!(matches!(c.conditional_installs[0].condition.conditions[1], Condition::Nested(_)));
        assert_eq!(c.conditional_installs[0].files[0].priority, -1);
    }

    #[test]
    fn utf16_with_and_without_bom() {
        let mut le = vec![0xFF, 0xFE];
        le.extend(SAMPLE.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(ModuleConfig::parse(&le).unwrap().module_name, "Sample");
        let be: Vec<u8> = SAMPLE.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(ModuleConfig::parse(&be).unwrap().steps.len(), 2);
    }

    #[test]
    fn latin1_fallback() {
        let mut bytes = b"<config><moduleName>Caf".to_vec();
        bytes.push(0xE9);
        bytes.extend_from_slice(b"</moduleName></config>");
        assert_eq!(ModuleConfig::parse(&bytes).unwrap().module_name, "Caf\u{e9}");
    }

    #[test]
    fn info_xml() {
        let info = Info::parse(b"<fomod><Name>My Mod</Name><Author>Me</Author><Version>1.2</Version><Id>42</Id><Website>https://x</Website></fomod>").unwrap();
        assert_eq!(info.name.as_deref(), Some("My Mod"));
        assert_eq!(info.id, Some(42));
        assert_eq!(info.version.as_deref(), Some("1.2"));
    }

    #[test]
    fn bad_group_type_fails() {
        let xml = r#"<config><installSteps><installStep name="a"><optionalFileGroups><group name="g" type="Pick"/></optionalFileGroups></installStep></installSteps></config>"#;
        assert!(ModuleConfig::parse(xml.as_bytes()).is_err());
    }
}
