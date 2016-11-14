use std::sync::Arc;
use std::iter::FromIterator;
use std::time::Duration;
use section::Section;
use section_runner::SectionRunner;

type ProgItem = (Arc<Section>, Duration);

pub struct Program {
    name: String,
    sequence: Vec<ProgItem>,
}

impl Program {
    pub fn new<S: Into<String>, I: IntoIterator<Item=ProgItem>>(name: S, sequence: I) -> Program {
        Program { name: name.into(), sequence: Vec::from_iter(sequence) }
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn run(&self, runner: &SectionRunner) {
        debug!("running program {}", self.name);
        for item in self.sequence.iter() {
            runner.run_section(item.0.clone(), item.1);
        }
    }
}

