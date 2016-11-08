#[macro_use]
extern crate log;
extern crate env_logger;

trait Section {
    fn name(&self) -> &String;
    fn set_state(&mut self, state: bool);
    fn state(&self) -> bool;
}

struct LogSection {
    name: String,
    state: bool,
}

impl LogSection {
    fn new<S: Into<String>>(name: S) -> LogSection {
        LogSection { name: name.into(), state: false }
    }
}

impl Section for LogSection {
    fn name(&self) -> &String {
        &self.name
    }

    fn set_state(&mut self, state: bool) {
        debug!("setting section {} section state to {}", self.name, state);
        self.state = state;
        
    }

    fn state(&self) -> bool {
        self.state
    }
}

fn main() {
    env_logger::init().unwrap();

    println!("Hello world!");
    let mut sections: Vec<Box<Section>>;
    // sections = vec![ Box::new(LogSection::new("Test 1")), Box::new(LogSection::new("Test 2")) ];
    sections = vec![ "Test 1", "Test 2" ].into_iter()
        .map(|name| Box::new(LogSection::new(name)) as Box<Section>)
        .collect();
    for section in sections.iter_mut() {
        println!("Section \"{}\"", section.name());
        section.set_state(false);
        section.set_state(true);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}
