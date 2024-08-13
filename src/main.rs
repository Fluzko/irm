use std::{
    cell::RefCell,
    fs, io,
    rc::{Rc, Weak},
    vec,
};

use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::Alignment,
    style::{Color, Style, Stylize},
    symbols::border,
    text::Line,
    widgets::{
        block::{Position, Title},
        Block, List, ListDirection, ListItem, ListState,
    },
    Frame,
};

mod tui;

#[derive(Debug, Clone, PartialEq, Eq)]
enum DirType {
    File,
    Dir,
    Symlink,
}

type NodeRef = Rc<Node>;

type Depth = usize;
type IsLastOfFolder = bool;
type Name = String;
type IsSelected = bool;
type TupleNode = (Name, DirType, Depth, IsLastOfFolder, IsSelected);

#[derive(Debug)]
struct DirTree {
    base_node: NodeRef,
}

impl DirTree {
    fn new(path: String) -> Self {
        Self {
            base_node: Node::new(path, DirType::Dir),
        }
    }

    fn to_array(&self) -> Vec<String> {
        let mut array = Vec::new();
        self.base_node.to_array(&mut array);
        array
    }

    fn find_node(&self, path: &str) -> Option<NodeRef> {
        let mut node: NodeRef = self.base_node.clone();

        if path == "." {
            return Some(node.clone());
        }

        for name in path.replace("./", "").split('/') {
            let _node = node.clone();
            let children = _node.children.borrow();
            let child = children.iter().find(|c| c.name == name);

            if let Some(child) = child {
                node = child.clone();
            } else {
                return None;
            }
        }
        Some(node.clone())
    }

    fn remove_node(&self, path: &str) {
        let node = self.find_node(path).unwrap().clone();
        let parent = node.parent.borrow();

        if parent.upgrade().is_none() {
            return;
        }
        parent
            .upgrade()
            .unwrap()
            .children
            .borrow_mut()
            .retain(|c| c.name != node.name);

        match node.type_ {
            DirType::Dir => fs::remove_dir_all(node.full_path()).unwrap(),
            DirType::File => fs::remove_file(node.full_path()).unwrap(),
            DirType::Symlink => todo!("implement symlinks"),
        }
    }

    fn to_enriched_array(&self, selected_nodes: &Vec<NodeRef>) -> Vec<TupleNode> {
        let mut items = Vec::new();
        self.base_node
            .to_enriched_array(&mut items, selected_nodes, 0, false);
        items
    }
}

#[derive(Debug, Clone)]
struct Node {
    name: String,
    type_: DirType,
    parent: RefCell<Weak<Node>>,
    children: RefCell<Vec<NodeRef>>,
}

impl Node {
    fn new(name: String, type_: DirType) -> NodeRef {
        Rc::new(Node {
            name,
            type_,
            parent: RefCell::new(Weak::new()),
            children: RefCell::new(Vec::new()),
        })
    }

    fn add_child(parent: Rc<Node>, child: NodeRef) {
        parent.children.borrow_mut().push(child.clone());
        *child.parent.borrow_mut() = Rc::downgrade(&parent);
    }

    fn scan_dir(node: NodeRef) -> io::Result<()> {
        let path = node.full_path();

        let entries = fs::read_dir(path)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();

            let type_ = if path.is_dir() {
                DirType::Dir
            } else if path.is_symlink() {
                DirType::Symlink
            } else {
                DirType::File
            };

            let child = Node::new(name, type_);

            Node::add_child(node.clone(), child);
        }

        Ok(())
    }

    fn full_path(&self) -> String {
        match self.parent.borrow() {
            parent if parent.upgrade().is_none() => self.name.clone(),
            parent => {
                let parent = parent.upgrade().unwrap();
                let parent_path = parent.full_path();
                format!("{}/{}", parent_path, self.name)
            }
        }
    }

    fn to_array(&self, array: &mut Vec<String>) {
        let full_path = self.full_path();
        array.push(full_path);
        for child in self.children.borrow().iter() {
            child.to_array(array);
        }
    }

    fn is_parent_selected(&self, selected_nodes: &[NodeRef]) -> bool {
        let parent = self.parent.borrow();
        match parent.upgrade() {
            None => false,
            Some(parent) => parent.is_selected(selected_nodes),
        }
    }

    fn is_selected(&self, selected_nodes: &[NodeRef]) -> bool {
        selected_nodes.iter().any(|node| {
            node.full_path() == self.full_path() || self.is_parent_selected(selected_nodes)
        })
    }

    fn to_enriched_array(
        &self,
        items: &mut Vec<TupleNode>,
        selected_nodes: &Vec<NodeRef>,
        depth: usize,
        is_last: bool,
    ) {
        let tuple = (
            self.name.clone(),
            self.type_.clone(),
            depth,
            is_last,
            self.is_selected(selected_nodes),
        );
        items.push(tuple);

        let children = self.children.clone();
        let len = children.borrow().len();

        for (i, child) in children.borrow().iter().enumerate() {
            let is_last = i == len - 1;
            child.to_enriched_array(items, selected_nodes, depth + 1, is_last);
        }
    }
}

#[derive(Debug)]
pub struct App {
    selected: Vec<NodeRef>,
    hovered: ListState,
    dir_tree: DirTree,
    exit: bool,
}

impl App {
    pub fn run(&mut self, terminal: &mut tui::Tui) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn handle_events(&mut self) -> io::Result<()> {
        match event::read()? {
            // it's important to check that the event is a key press event as
            // crossterm also emits key release and repeat events on Windows.
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event)
            }
            _ => {}
        };
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.handle_exit()
            }
            KeyCode::Char('q') => self.handle_exit(),
            KeyCode::Char(' ') => self.handle_select_dir(),
            KeyCode::Up => self.handle_hover_up(),
            KeyCode::Down => self.handle_hover_down(),
            KeyCode::Enter => self.handle_open_dir(),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.handle_clear_all()
            }
            KeyCode::Char('r') => self.handle_clear_hovered(),
            _ => {}
        }
    }

    fn default() -> Self {
        let current_dir = ".".to_string();
        let dir_tree = DirTree::new(current_dir);
        let hovered = ListState::default().with_selected(Some(0));

        Self {
            hovered,
            selected: Vec::new(),
            dir_tree,
            exit: false,
        }
    }

    fn handle_exit(&mut self) {
        self.exit = true;
    }

    fn handle_select_dir(&mut self) {
        let arr = self.dir_tree.to_array();
        let idx = self.hovered.selected().unwrap();
        let node_path = arr[idx].clone();
        let node = self.dir_tree.find_node(&node_path).unwrap();

        if self.selected.iter().any(|x| x.full_path() == node_path) {
            self.selected.retain(|x| x.full_path() != node_path);
        } else {
            self.selected.push(node);
        }
    }

    fn handle_open_dir(&mut self) {
        let arr = self.dir_tree.to_array();
        let idx = self.hovered.selected().unwrap();
        let node_path = arr[idx].clone();
        let node = self.dir_tree.find_node(&node_path).unwrap();

        if node.children.borrow().is_empty() && node.type_ == DirType::Dir {
            Node::scan_dir(node.clone()).unwrap();
        }
    }

    fn handle_hover_down(&mut self) {
        let i = match self.hovered.selected() {
            None => 0,
            Some(i) => {
                if i >= self.dir_tree.to_array().len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
        };

        self.hovered.select(Some(i));
    }

    fn handle_hover_up(&mut self) {
        let i = match self.hovered.selected() {
            Some(i) => {
                if i == 0 {
                    self.dir_tree.to_array().len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.hovered.select(Some(i));
    }

    fn handle_clear_hovered(&mut self) {
        let arr = self.dir_tree.to_array();
        let idx = self.hovered.selected().unwrap();
        let node_path = arr[idx].clone();

        self.dir_tree.remove_node(&node_path);
    }

    fn handle_clear_all(&mut self) {
        self.selected.iter().for_each(|node| {
            self.dir_tree.remove_node(&node.full_path());
        });
    }
}

impl App {
    fn draw(&mut self, f: &mut Frame) {
        let title = Title::from(" Interactive file remover ".bold());

        let instructions = Title::from(Line::from(vec![
            " Move: ".into(),
            "<Up/Down>".blue().bold(),
            " Open dir: ".into(),
            "<Enter>".blue().bold(),
            " Select: ".into(),
            "<Space>".blue().bold(),
            " Remove all: ".into(),
            "<Shift + R> ".red().bold(),
            "Remove: ".into(),
            "<R> ".red().bold(),
            " Quit: ".into(),
            "<Q> ".blue().bold(),
        ]));
        let block = Block::bordered()
            .title(title.alignment(Alignment::Center))
            .title(
                instructions
                    .alignment(Alignment::Center)
                    .position(Position::Bottom),
            )
            .border_set(border::THICK);

        let enriched = self.dir_tree.to_enriched_array(&self.selected);
        let items = enriched
            .iter()
            .map(|(name, type_, depth, is_last, is_selected)| {
                let type_prefix = match type_ {
                    DirType::Dir => "📁",
                    DirType::File => "📄",
                    DirType::Symlink => "🔗",
                };

                let list_prefix = if *is_last { "└─" } else { "├─" };
                let depth_prefix = "│ ".repeat(*depth);

                let formatted = format!("{depth_prefix}{list_prefix} {type_prefix} {name}");

                let li = ListItem::new(formatted);

                if *is_selected {
                    li.style(Style::default().fg(Color::Red))
                } else {
                    li
                }
            });

        let list = List::new(items)
            .block(Block::bordered().title("Directories"))
            .style(Style::default().fg(Color::White))
            .highlight_symbol("▶️")
            .repeat_highlight_symbol(true)
            .block(block)
            .direction(ListDirection::TopToBottom);

        f.render_stateful_widget(list, f.size(), &mut self.hovered);
    }
}

fn main() -> io::Result<()> {
    let mut terminal = tui::init()?;
    let mut app = App::default();
    let app_result = app.run(&mut terminal);
    tui::restore()?;
    app_result
}
