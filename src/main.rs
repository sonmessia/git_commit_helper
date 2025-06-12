use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
    },
    Frame, Terminal,
};
use std::{
    collections::HashMap,
    io,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq)]
pub enum FileStatus {
    Untracked,
    Modified,
    Staged,
    Added,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone)]
pub struct GitFile {
    pub path: String,
    pub status: FileStatus,
    pub staged: bool,
}

#[derive(Debug, PartialEq)]
pub enum AppMode {
    FileList,
    DiffView,
    CommitMessage,
    Help,
}

#[derive(Debug)]
pub struct GitStatus {
    pub current_branch: String,
    pub ahead: i32,
    pub behind: i32,
    pub files: Vec<GitFile>,
}

#[derive(Debug)]
pub struct App {
    pub mode: AppMode,
    pub files: Vec<GitFile>,
    pub selected_file: usize,
    pub file_list_state: ListState,
    pub commit_message: String,
    pub commit_prefix: String,
    pub commit_prefixes: Vec<String>,
    pub selected_prefix: usize,
    pub git_status: GitStatus,
    pub diff_content: String,
    pub notification: Option<(String, Instant)>,
    pub should_quit: bool,
    pub cursor_position: usize,
}

impl Default for App {
    fn default() -> App {
        let mut app = App {
            mode: AppMode::FileList,
            files: Vec::new(),
            selected_file: 0,
            file_list_state: ListState::default(),
            commit_message: String::new(),
            commit_prefix: String::new(),
            commit_prefixes: vec![
                "feat: ".to_string(),
                "fix: ".to_string(),
                "docs: ".to_string(),
                "style: ".to_string(),
                "refactor: ".to_string(),
                "test: ".to_string(),
                "chore: ".to_string(),
            ],
            selected_prefix: 0,
            git_status: GitStatus {
                current_branch: String::new(),
                ahead: 0,
                behind: 0,
                files: Vec::new(),
            },
            diff_content: String::new(),
            notification: None,
            should_quit: false,
            cursor_position: 0,
        };
        app.file_list_state.select(Some(0));
        app
    }
}

impl App {
    pub fn run<B: Backend>(mut self, terminal: &mut Terminal<B>) -> io::Result<()> {
        self.refresh_git_status();
        
        loop {
            terminal.draw(|f| self.ui(f))?;

            if self.should_quit {
                break;
            }

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_input(key.code);
                    }
                }
            }

            // Clear expired notifications
            if let Some((_, time)) = &self.notification {
                if time.elapsed() > Duration::from_secs(3) {
                    self.notification = None;
                }
            }
        }

        Ok(())
    }

    fn handle_input(&mut self, key: KeyCode) {
        match self.mode {
            AppMode::FileList => self.handle_file_list_input(key),
            AppMode::DiffView => self.handle_diff_view_input(key),
            AppMode::CommitMessage => self.handle_commit_message_input(key),
            AppMode::Help => self.handle_help_input(key),
        }
    }

    fn handle_file_list_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('h') | KeyCode::F(1) => self.mode = AppMode::Help,
            KeyCode::Char('r') => self.refresh_git_status(),
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.files.is_empty() {
                    self.selected_file = (self.selected_file + 1) % self.files.len();
                    self.file_list_state.select(Some(self.selected_file));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.files.is_empty() {
                    self.selected_file = if self.selected_file == 0 {
                        self.files.len() - 1
                    } else {
                        self.selected_file - 1
                    };
                    self.file_list_state.select(Some(self.selected_file));
                }
            }
            KeyCode::Char(' ') => self.toggle_stage_file(),
            KeyCode::Char('d') => {
                if !self.files.is_empty() {
                    self.show_diff();
                }
            }
            KeyCode::Char('c') => {
                if self.has_staged_files() {
                    self.mode = AppMode::CommitMessage;
                } else {
                    self.show_notification("No staged files to commit".to_string());
                }
            }
            KeyCode::Char('p') => self.push_to_remote(),
            _ => {}
        }
    }

    fn handle_diff_view_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc | KeyCode::Char('q') => self.mode = AppMode::FileList,
            _ => {}
        }
    }

    fn handle_commit_message_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => self.mode = AppMode::FileList,
            KeyCode::Enter => {
                if !self.commit_message.trim().is_empty() {
                    self.perform_commit();
                    self.mode = AppMode::FileList;
                } else {
                    self.show_notification("Commit message cannot be empty".to_string());
                }
            }
            KeyCode::Char(c) => {
                self.commit_message.insert(self.cursor_position, c);
                self.cursor_position += 1;
            }
            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    self.commit_message.remove(self.cursor_position - 1);
                    self.cursor_position -= 1;
                }
            }
            KeyCode::Delete => {
                if self.cursor_position < self.commit_message.len() {
                    self.commit_message.remove(self.cursor_position);
                }
            }
            KeyCode::Left => {
                if self.cursor_position > 0 {
                    self.cursor_position -= 1;
                }
            }
            KeyCode::Right => {
                if self.cursor_position < self.commit_message.len() {
                    self.cursor_position += 1;
                }
            }
            KeyCode::Home => self.cursor_position = 0,
            KeyCode::End => self.cursor_position = self.commit_message.len(),
            KeyCode::Tab => {
                if self.commit_message.is_empty() {
                    self.selected_prefix = (self.selected_prefix + 1) % self.commit_prefixes.len();
                    self.commit_message = self.commit_prefixes[self.selected_prefix].clone();
                    self.cursor_position = self.commit_message.len();
                }
            }
            _ => {}
        }
    }

    fn handle_help_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') => self.mode = AppMode::FileList,
            _ => {}
        }
    }

    fn refresh_git_status(&mut self) {
        self.git_status = self.get_git_status();
        self.files = self.git_status.files.clone();
        
        if self.files.is_empty() {
            self.selected_file = 0;
            self.file_list_state.select(None);
        } else {
            self.selected_file = self.selected_file.min(self.files.len() - 1);
            self.file_list_state.select(Some(self.selected_file));
        }
    }

    fn get_git_status(&self) -> GitStatus {
        let mut status = GitStatus {
            current_branch: self.get_current_branch(),
            ahead: 0,
            behind: 0,
            files: Vec::new(),
        };

        // Get ahead/behind counts
        if let Ok(output) = Command::new("git")
            .args(&["rev-list", "--left-right", "--count", "HEAD...@{u}"])
            .output()
        {
            if output.status.success() {
                let counts = String::from_utf8_lossy(&output.stdout);
                let parts: Vec<&str> = counts.trim().split('\t').collect();
                if parts.len() == 2 {
                    status.ahead = parts[0].parse().unwrap_or(0);
                    status.behind = parts[1].parse().unwrap_or(0);
                }
            }
        }

        // Get file status
        if let Ok(output) = Command::new("git")
            .args(&["status", "--porcelain"])
            .output()
        {
            if output.status.success() {
                let output_str = String::from_utf8_lossy(&output.stdout);
                for line in output_str.lines() {
                    if line.len() >= 3 {
                        let staged_status = line.chars().nth(0).unwrap_or(' ');
                        let unstaged_status = line.chars().nth(1).unwrap_or(' ');
                        let path = line[3..].to_string();

                        let file_status = match (staged_status, unstaged_status) {
                            ('A', _) => FileStatus::Added,
                            ('M', _) => FileStatus::Staged,
                            ('D', _) => FileStatus::Deleted,
                            ('R', _) => FileStatus::Renamed,
                            ('?', '?') => FileStatus::Untracked,
                            (_, 'M') => FileStatus::Modified,
                            (_, 'D') => FileStatus::Deleted,
                            _ => FileStatus::Modified,
                        };

                        let staged = staged_status != ' ' && staged_status != '?';

                        status.files.push(GitFile {
                            path,
                            status: file_status,
                            staged,
                        });
                    }
                }
            }
        }

        status
    }

    fn get_current_branch(&self) -> String {
        if let Ok(output) = Command::new("git")
            .args(&["branch", "--show-current"])
            .output()
        {
            if output.status.success() {
                return String::from_utf8_lossy(&output.stdout).trim().to_string();
            }
        }
        "unknown".to_string()
    }

    fn toggle_stage_file(&mut self) {
        if self.files.is_empty() {
            return;
        }

        let file = &self.files[self.selected_file];
        
        if file.staged {
            self.unstage_file(&file.path);
        } else {
            self.stage_file(&file.path);
        }
        
        self.refresh_git_status();
    }

    fn stage_file(&self, path: &str) {
        let _ = Command::new("git")
            .args(&["add", path])
            .output();
    }

    fn unstage_file(&self, path: &str) {
        let _ = Command::new("git")
            .args(&["reset", "HEAD", path])
            .output();
    }

    fn show_diff(&mut self) {
        if self.files.is_empty() {
            return;
        }

        let file = &self.files[self.selected_file];
        let diff_args = if file.staged {
            vec!["diff", "--staged", &file.path]
        } else {
            vec!["diff", &file.path]
        };

        if let Ok(output) = Command::new("git").args(&diff_args).output() {
            if output.status.success() {
                self.diff_content = String::from_utf8_lossy(&output.stdout).to_string();
                self.mode = AppMode::DiffView;
            }
        }
    }

    fn has_staged_files(&self) -> bool {
        self.files.iter().any(|f| f.staged)
    }

    fn perform_commit(&mut self) {
        if let Ok(output) = Command::new("git")
            .args(&["commit", "-m", &self.commit_message])
            .output()
        {
            if output.status.success() {
                self.show_notification("Commit successful".to_string());
                self.commit_message.clear();
                self.cursor_position = 0;
                self.refresh_git_status();
            } else {
                let error = String::from_utf8_lossy(&output.stderr);
                self.show_notification(format!("Commit failed: {}", error));
            }
        }
    }

    fn push_to_remote(&mut self) {
        if let Ok(output) = Command::new("git")
            .args(&["push", "origin", &self.git_status.current_branch])
            .output()
        {
            if output.status.success() {
                self.show_notification("Push successful".to_string());
                self.refresh_git_status();
            } else {
                let error = String::from_utf8_lossy(&output.stderr);
                self.show_notification(format!("Push failed: {}", error));
            }
        }
    }

    fn show_notification(&mut self, message: String) {
        self.notification = Some((message, Instant::now()));
    }

    fn ui(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Main content
                Constraint::Length(1), // Status bar
            ])
            .split(f.area());

        self.render_header(f, chunks[0]);
        
        match self.mode {
            AppMode::FileList => self.render_file_list(f, chunks[1]),
            AppMode::DiffView => self.render_diff_view(f, chunks[1]),
            AppMode::CommitMessage => self.render_commit_message(f, chunks[1]),
            AppMode::Help => self.render_help(f, chunks[1]),
        }

        self.render_status_bar(f, chunks[2]);

        if let Some((message, _)) = &self.notification {
            self.render_notification(f, message);
        }
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let ahead_behind = if self.git_status.ahead > 0 || self.git_status.behind > 0 {
            format!(" (↑{} ↓{})", self.git_status.ahead, self.git_status.behind)
        } else {
            String::new()
        };

        let header_text = format!(
            "Git Commit Helper - Branch: {}{} - Files: {}",
            self.git_status.current_branch,
            ahead_behind,
            self.files.len()
        );

        let header = Paragraph::new(header_text)
            .style(Style::default().fg(Color::Yellow))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));

        f.render_widget(header, area);
    }

    fn render_file_list(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .files
            .iter()
            .map(|file| {
                let status_char = match file.status {
                    FileStatus::Untracked => "?",
                    FileStatus::Modified => "M",
                    FileStatus::Added => "A",
                    FileStatus::Deleted => "D",
                    FileStatus::Renamed => "R",
                    FileStatus::Staged => "M",
                };

                let staged_char = if file.staged { "●" } else { "○" };
                let color = if file.staged { Color::Green } else { Color::Red };

                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{} {} ", staged_char, status_char),
                        Style::default().fg(color),
                    ),
                    Span::raw(&file.path),
                ]))
            })
            .collect();

        let files_list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Files"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");

        f.render_stateful_widget(files_list, area, &mut self.file_list_state);
    }

    fn render_diff_view(&self, f: &mut Frame, area: Rect) {
        let diff = Paragraph::new(self.diff_content.as_str())
            .block(Block::default().borders(Borders::ALL).title("Diff"))
            .wrap(Wrap { trim: true });

        f.render_widget(diff, area);
    }

    fn render_commit_message(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);

        // Prefix suggestions
        let prefixes: Vec<ListItem> = self
            .commit_prefixes
            .iter()
            .enumerate()
            .map(|(i, prefix)| {
                let style = if i == self.selected_prefix {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(prefix.as_str()).style(style)
            })
            .collect();

        let prefix_list = List::new(prefixes)
            .block(Block::default().borders(Borders::ALL).title("Prefixes (Tab to cycle)"));

        f.render_widget(prefix_list, chunks[0]);

        // Commit message input
        let message_len = self.commit_message.chars().count();
        let title = format!("Commit Message ({})", message_len);
        let color = if message_len > 50 { Color::Red } else { Color::White };

        let input = Paragraph::new(self.commit_message.as_str())
            .style(Style::default().fg(color))
            .block(Block::default().borders(Borders::ALL).title(title));

        f.render_widget(input, chunks[1]);

        // Set cursor position
        f.set_cursor_position((
            chunks[1].x + self.cursor_position as u16 + 1,
            chunks[1].y + 1,
        ));
    }

    fn render_help(&self, f: &mut Frame, area: Rect) {
        let help_text = vec![
            "Git Commit Helper - Keyboard Shortcuts",
            "",
            "File List Mode:",
            "  ↑/k, ↓/j     - Navigate files",
            "  Space        - Stage/unstage file",
            "  d            - View diff of selected file",
            "  c            - Start commit (if files are staged)",
            "  p            - Push to remote",
            "  r            - Refresh git status",
            "  h/F1         - Show this help",
            "  q            - Quit",
            "",
            "Commit Message Mode:",
            "  Tab          - Cycle through commit prefixes",
            "  Enter        - Commit changes",
            "  Esc          - Cancel commit",
            "",
            "Diff View Mode:",
            "  Esc/q        - Return to file list",
            "",
            "Press Esc or q to close this help",
        ];

        let help = Paragraph::new(help_text.join("\n"))
            .block(Block::default().borders(Borders::ALL).title("Help"))
            .wrap(Wrap { trim: true });

        f.render_widget(help, area);
    }

    fn render_status_bar(&self, f: &mut Frame, area: Rect) {
        let mode_text = match self.mode {
            AppMode::FileList => "FILE LIST",
            AppMode::DiffView => "DIFF VIEW",
            AppMode::CommitMessage => "COMMIT MESSAGE",
            AppMode::Help => "HELP",
        };

        let status_text = format!("Mode: {} | Press 'h' for help | 'q' to quit", mode_text);
        let status = Paragraph::new(status_text)
            .style(Style::default().fg(Color::White).bg(Color::Blue));

        f.render_widget(status, area);
    }

    fn render_notification(&self, f: &mut Frame, message: &str) {
        let area = Rect {
            x: f.area().width / 4,
            y: f.area().height / 2,
            width: f.area().width / 2,
            height: 3,
        };

        f.render_widget(Clear, area);
        
        let notification = Paragraph::new(message)
            .style(Style::default().fg(Color::White).bg(Color::Red))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));

        f.render_widget(notification, area);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run it
    let app = App::default();
    let res = app.run(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}
