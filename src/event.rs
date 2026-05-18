use crate::state::{LayoutIndex, WindowAddr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event<'a> {
    ActiveWindow {
        class_name: &'a str,
    },
    ActiveWindowV2 {
        addr: WindowAddr,
    },
    EmptyActiveWindow,
    CloseWindow {
        addr: WindowAddr,
    },
    ActiveLayout {
        keyboard: &'a str,
        layout_name: &'a str,
    },
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    MissingSeparator,
    BadWindowAddress,
    BadActiveLayout,
}

pub fn parse_line(line: &str) -> Result<Event<'_>, ParseError> {
    let line = line.trim_end_matches(['\n', '\r']);
    let Some((name, data)) = line.split_once(">>") else {
        return Err(ParseError::MissingSeparator);
    };

    match name {
        "activewindow" => {
            let class_name = data
                .split_once(',')
                .map_or(data, |(class_name, _)| class_name);
            Ok(Event::ActiveWindow { class_name })
        }
        "activewindowv2" => {
            if data.is_empty() {
                Ok(Event::EmptyActiveWindow)
            } else {
                Ok(Event::ActiveWindowV2 {
                    addr: parse_window_addr(data)?,
                })
            }
        }
        "closewindow" => Ok(Event::CloseWindow {
            addr: parse_window_addr(data)?,
        }),
        "activelayout" => {
            let Some((keyboard, layout_name)) = data.split_once(',') else {
                return Err(ParseError::BadActiveLayout);
            };
            Ok(Event::ActiveLayout {
                keyboard,
                layout_name,
            })
        }
        _ => Ok(Event::Ignored),
    }
}

pub fn parse_window_addr(input: &str) -> Result<WindowAddr, ParseError> {
    let input = input.trim_start_matches("0x");
    u64::from_str_radix(input, 16)
        .map(WindowAddr)
        .map_err(|_| ParseError::BadWindowAddress)
}

pub fn parse_layout_index(input: u64) -> Option<LayoutIndex> {
    LayoutIndex::try_from(input).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_active_window_class() {
        assert_eq!(
            parse_line("activewindow>>firefox,Some title\n").unwrap(),
            Event::ActiveWindow {
                class_name: "firefox"
            }
        );
    }

    #[test]
    fn parses_active_window_v2_address() {
        assert_eq!(
            parse_line("activewindowv2>>55b0cd60fa30\n").unwrap(),
            Event::ActiveWindowV2 {
                addr: WindowAddr(0x55b0cd60fa30)
            }
        );
    }

    #[test]
    fn parses_empty_active_window() {
        assert_eq!(
            parse_line("activewindowv2>>\n").unwrap(),
            Event::EmptyActiveWindow
        );
    }

    #[test]
    fn parses_close_window_with_prefix() {
        assert_eq!(
            parse_line("closewindow>>0x55b0cd60fa30\n").unwrap(),
            Event::CloseWindow {
                addr: WindowAddr(0x55b0cd60fa30)
            }
        );
    }

    #[test]
    fn parses_active_layout_with_commas_in_layout_name() {
        assert_eq!(
            parse_line("activelayout>>kbd,English (US, intl., with dead keys)\n").unwrap(),
            Event::ActiveLayout {
                keyboard: "kbd",
                layout_name: "English (US, intl., with dead keys)"
            }
        );
    }

    #[test]
    fn rejects_bad_addresses() {
        assert_eq!(
            parse_line("activewindowv2>>not-hex").unwrap_err(),
            ParseError::BadWindowAddress
        );
    }
}
