//! Items related to reading configuration from files
use super::*;

#[derive(Serialize, Deserialize, Debug)]
/// A (usually JSON) config for a `Section`.
pub struct SectionConfig {
    name: String,
}

impl SectionConfig {
    /// Converts a `SectionConfig` to an `Arc<Section>` based on the config. The actual struct
    /// which impls `Section` may vary at runtime.
    pub fn to_section(&self) -> SectionRef {
        // TODO: Check some environment variable and change the type of section created
        Arc::new(LogSection::new_noop(&self.name as &str)) as SectionRef
    }

    /// Converts an `Arc<Section>` to a `SectionConfig`.
    pub fn from_section(section: &Arc<Section>) -> Self {
        SectionConfig {
            name: section.name().clone(),
        }
    }
}

quick_error!{
    /// An error that can be returned from config related methods
    #[derive(Debug)]
    pub enum ConfigError {
        /// The specified section index was out of range
        SectionOutOfRange(sec_idx: usize) {
            description("Section index was out of range")
            display("Section index {} is out of range", sec_idx)
        }
    }
}

/// A result that can be returned from config related methods
pub type ConfigResult<T> = Result<T, ConfigError>;

#[derive(Serialize, Deserialize, Debug)]
struct ProgItemConfig {
    section: usize,
    #[serde(deserialize_with="util::deserialize_duration")]
    duration: Duration,
}

impl ProgItemConfig {
    fn to_prog_item(&self, sections: &[SectionRef]) -> ConfigResult<ProgItem> {
        if self.section >= sections.len() {
            Err(ConfigError::SectionOutOfRange(self.section))
        } else {
            Ok(ProgItem::new(sections[self.section].clone(), self.duration))
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct ProgramConfig {
    name: String,
    sequence: Box<[ProgItemConfig]>,
    schedule: Schedule,
}

impl ProgramConfig {
    fn to_program(&self, sections: &[SectionRef]) -> ConfigResult<ProgramRef> {
        let sequence: ConfigResult<Vec<ProgItem>> = self.sequence
            .iter()
            .map(|prog_item| prog_item.to_prog_item(sections))
            .collect();
        let sequence = try!(sequence);
        Ok(Program::new(self.name.clone(), sequence, self.schedule.clone()))
    }
}

#[derive(Serialize, Deserialize, Debug)]
/// The configuration format for rinklers
pub struct Config {
    sections: Vec<SectionConfig>,
    programs: Vec<ProgramConfig>,
}

impl Config {
    /// Gets the `Vec` of `Section`s as specified in the config
    pub fn to_sections(&self) -> Vec<SectionRef> {
        self.sections
            .iter()
            .map(|sec_conf| sec_conf.to_section())
            .collect()
    }

    /// Gets the `Vec` of `Programs`s as specified in the config. Gets sections by index from
    /// `sections`
    pub fn to_programs(&self, sections: &[SectionRef]) -> ConfigResult<Vec<ProgramRef>> {
        self.programs
            .iter()
            .map(|program_conf| program_conf.to_program(sections))
            .collect::<ConfigResult<_>>()
    }
}
