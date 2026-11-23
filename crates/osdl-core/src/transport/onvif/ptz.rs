use super::{xml_escape, DiscoveredEndpoints, OnvifTransport};
use jiff::fmt::temporal::SpanParser;
use jiff::SpanRelativeTo;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::ops::RangeInclusive;
use std::time::Duration;

impl OnvifTransport {
    pub(super) async fn discover_ptz_timeout_range(
        &self,
        endpoints: &DiscoveredEndpoints,
    ) -> Result<PtzTimeoutRange, String> {
        let configuration = match &endpoints.ptz_configuration_token {
            Some(token) => Some(token.clone()),
            None if self.inner.profile_token_override.is_some() => {
                let profile = xml_escape(&endpoints.profile_token);
                let body = format!(
                    "<trt:GetProfile><trt:ProfileToken>{profile}</trt:ProfileToken></trt:GetProfile>"
                );
                let response = self
                    .soap(&endpoints.media_url, &body)
                    .await
                    .map_err(|e| format!("GetProfile: {e}"))?;
                parse_profile(&response, b"Profile")
                    .filter(|(token, _)| *token == endpoints.profile_token)
                    .and_then(|(_, config)| config)
            }
            None => None,
        }
        .ok_or("selected ONVIF profile has no PTZ configuration token")?;

        let token = xml_escape(&configuration);
        let body = format!(
            "<tptz:GetConfigurationOptions><tptz:ConfigurationToken>{token}</tptz:ConfigurationToken></tptz:GetConfigurationOptions>"
        );
        let response = self
            .soap(&endpoints.ptz_url, &body)
            .await
            .map_err(|e| format!("GetConfigurationOptions: {e}"))?;
        PtzTimeoutRange::parse(&response)
            .ok_or_else(|| "GetConfigurationOptions: missing or invalid PTZTimeout range".into())
    }
}

pub(super) struct PtzTimeoutRange(RangeInclusive<Duration>);

impl PtzTimeoutRange {
    fn parse(response: &str) -> Option<Self> {
        let (_, options) = find_element(response, b"PTZConfigurationOptions")?;
        let (_, timeout) = find_element(options, b"PTZTimeout")?;
        let duration = |tag| {
            let (_, text) = find_element(timeout, tag)?;
            let span = SpanParser::new().parse_span(text.trim()).ok()?;
            span.to_duration(SpanRelativeTo::days_are_24_hours())
                .ok()?
                .try_into()
                .ok()
        };
        let min = duration(b"Min")?;
        let max = duration(b"Max")?;
        (min <= max).then_some(Self(min..=max))
    }

    pub(super) fn validate(&self, duration: Duration) -> Result<(), String> {
        if self.0.contains(&duration) {
            return Ok(());
        }
        Err(format!(
            "requested PTZ duration {duration:?} is outside camera PTZ timeout range {:?}–{:?}",
            self.0.start(),
            self.0.end()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reported_timeout_bounds_are_inclusive() {
        let response = "<PTZConfigurationOptions><PTZTimeout><Min>PT00H00M01S</Min>\
                        <Max>PT00H01M00S</Max></PTZTimeout></PTZConfigurationOptions>";
        let range = PtzTimeoutRange::parse(response).unwrap();
        assert!(range.validate(Duration::from_millis(500)).is_err());
        assert!(range.validate(Duration::from_secs(1)).is_ok());
        assert!(range.validate(Duration::from_secs(60)).is_ok());
        assert!(range.validate(Duration::from_millis(60_001)).is_err());
    }
}

pub(super) fn parse_profile(body: &str, element: &[u8]) -> Option<(String, Option<String>)> {
    let (profile, contents) = find_element(body, element)?;
    let token = element_token(&profile)?;
    let config = find_element(contents, b"PTZConfiguration")
        .and_then(|(configuration, _)| element_token(&configuration));
    Some((token, config))
}

fn element_token(element: &BytesStart<'_>) -> Option<String> {
    element
        .try_get_attribute("token")
        .ok()??
        .unescape_value()
        .ok()
        .filter(|token| !token.is_empty())
        .map(|token| token.into_owned())
}

// Borrow the first matching element's contents so child lookups stay within it.
fn find_element<'a>(body: &'a str, name: &[u8]) -> Option<(BytesStart<'a>, &'a str)> {
    let mut reader = Reader::from_str(body);
    loop {
        match reader.read_event().ok()? {
            Event::Start(element) if element.local_name().as_ref() == name => {
                let inner = reader.read_to_end(element.name()).ok()?;
                return Some((element, &body[inner.start as usize..inner.end as usize]));
            }
            Event::Empty(element) if element.local_name().as_ref() == name => {
                return Some((element, ""));
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}
