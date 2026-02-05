//! TUI UI Rendering
//!
//! Renders the terminal UI using ratatui with 4 zones:
//! - Header: pair, spread, position status
//! - Orderbooks: Vest and Paradex prices side by side
//! - Stats: entry, PnL, latency, trades, uptime
//! - Logs: scrollable log entries

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use super::app::AppState;

/// Main draw function - renders the entire UI
pub fn draw(frame: &mut Frame, state: &AppState) {
    // Create main layout: header, content, logs
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(6),  // Orderbooks
            Constraint::Length(4),  // Stats
            Constraint::Min(8),     // Logs
        ])
        .split(frame.area());

    draw_header(frame, chunks[0], state);
    draw_orderbooks(frame, chunks[1], state);
    draw_stats(frame, chunks[2], state);
    draw_logs(frame, chunks[3], state);
}

/// Draw header with pair, spread, and position status
fn draw_header(frame: &mut Frame, area: Rect, state: &AppState) {
    // Format spread with direction
    let spread_text = if let Some(dir) = &state.spread_direction {
        format!("{:.2}% ({:?})", state.current_spread_pct * 100.0, dir)
    } else {
        format!("{:.2}%", state.current_spread_pct * 100.0)
    };
    
    // Spread color: green if above threshold, white otherwise
    let spread_color = if state.current_spread_pct >= state.spread_entry_threshold {
        Color::Green
    } else {
        Color::White
    };
    
    // Position status
    let (pos_text, pos_color) = if state.position_open {
        ("● OPEN", Color::Green)
    } else {
        ("○ IDLE", Color::DarkGray)
    };
    
    let header = Paragraph::new(Line::from(vec![
        Span::styled(&state.pair, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("  │  Spread: "),
        Span::styled(spread_text, Style::default().fg(spread_color)),
        Span::raw("  │  Position: "),
        Span::styled(pos_text, Style::default().fg(pos_color).add_modifier(Modifier::BOLD)),
    ]))
    .block(Block::default().borders(Borders::ALL).title("HFT Bot"));
    
    frame.render_widget(header, area);
}

/// Draw orderbooks side by side
fn draw_orderbooks(frame: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    
    // Vest orderbook
    let vest_mid = (state.vest_best_bid + state.vest_best_ask) / 2.0;
    let vest_content = vec![
        Line::from(vec![
            Span::raw("Ask: "),
            Span::styled(format!("${:.2}", state.vest_best_ask), Style::default().fg(Color::Red)),
        ]),
        Line::from(vec![
            Span::raw("Bid: "),
            Span::styled(format!("${:.2}", state.vest_best_bid), Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::raw("Mid: "),
            Span::styled(format!("${:.2}", vest_mid), Style::default().fg(Color::White)),
        ]),
    ];
    let vest_block = Paragraph::new(vest_content)
        .block(Block::default().borders(Borders::ALL).title("VEST"));
    frame.render_widget(vest_block, chunks[0]);
    
    // Paradex orderbook
    let paradex_mid = (state.paradex_best_bid + state.paradex_best_ask) / 2.0;
    let paradex_content = vec![
        Line::from(vec![
            Span::raw("Ask: "),
            Span::styled(format!("${:.2}", state.paradex_best_ask), Style::default().fg(Color::Red)),
        ]),
        Line::from(vec![
            Span::raw("Bid: "),
            Span::styled(format!("${:.2}", state.paradex_best_bid), Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::raw("Mid: "),
            Span::styled(format!("${:.2}", paradex_mid), Style::default().fg(Color::White)),
        ]),
    ];
    let paradex_block = Paragraph::new(paradex_content)
        .block(Block::default().borders(Borders::ALL).title("PARADEX"));
    frame.render_widget(paradex_block, chunks[1]);
}

/// Draw stats panel
fn draw_stats(frame: &mut Frame, area: Rect, state: &AppState) {
    let entry_text = if let Some(entry) = state.entry_spread {
        format!("{:.2}%", entry * 100.0)
    } else {
        "-".to_string()
    };
    
    let pnl_color = if state.total_profit_pct >= 0.0 { Color::Green } else { Color::Red };
    let pnl_text = format!("{:+.2}%", state.total_profit_pct * 100.0);
    
    let latency_text = state.last_latency_ms
        .map(|l| format!("{}ms", l))
        .unwrap_or_else(|| "-".to_string());
    
    let line1 = Line::from(vec![
        Span::raw("Entry: "),
        Span::styled(entry_text, Style::default().fg(Color::Cyan)),
        Span::raw("  │  PnL: "),
        Span::styled(pnl_text, Style::default().fg(pnl_color)),
        Span::raw("  │  Latency: "),
        Span::styled(latency_text, Style::default().fg(Color::Yellow)),
        Span::raw("  │  Trades: "),
        Span::styled(format!("{}", state.trades_count), Style::default().fg(Color::White)),
    ]);
    
    let line2 = Line::from(vec![
        Span::raw("Uptime: "),
        Span::styled(state.uptime_str(), Style::default().fg(Color::Cyan)),
        Span::raw("  │  Size: "),
        Span::styled(format!("{} {}", state.position_size, state.pair), Style::default().fg(Color::White)),
        Span::raw("  │  Leverage: "),
        Span::styled(format!("{}x", state.leverage), Style::default().fg(Color::Yellow)),
    ]);
    
    let stats = Paragraph::new(vec![line1, line2])
        .block(Block::default().borders(Borders::ALL).title("Stats"));
    
    frame.render_widget(stats, area);
}

/// Draw scrollable log panel
fn draw_logs(frame: &mut Frame, area: Rect, state: &AppState) {
    let log_items: Vec<ListItem> = state.recent_logs
        .iter()
        .rev()  // Most recent first
        .skip(state.log_scroll_offset)
        .take(area.height.saturating_sub(2) as usize)  // Fit in area minus borders
        .map(|entry| {
            let level_color = match entry.level.as_str() {
                "ERROR" => Color::Red,
                "WARN" => Color::Yellow,
                "INFO" => Color::Cyan,
                "DEBUG" => Color::DarkGray,
                _ => Color::White,
            };
            
            ListItem::new(Line::from(vec![
                Span::styled(&entry.timestamp, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(
                    format!("{:5}", entry.level),
                    Style::default().fg(level_color),
                ),
                Span::raw(" "),
                Span::raw(&entry.message),
            ]))
        })
        .collect();
    
    let debug_indicator = if state.show_debug_logs { " [DEBUG ON]" } else { "" };
    let title = format!("Logs (↑/↓ scroll, L=debug){}", debug_indicator);
    
    let logs = List::new(log_items)
        .block(Block::default().borders(Borders::ALL).title(title));
    
    frame.render_widget(logs, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_app_state_for_ui() {
        let state = AppState::new("BTC-PERP".into(), 0.15, 0.05, 0.001, 10);
        // Just verify state can be used for UI - actual rendering requires terminal
        assert_eq!(state.pair, "BTC-PERP");
    }
}
