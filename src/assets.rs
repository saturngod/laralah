use anyhow::Result;
use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

pub const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icons/laralah.png");
#[cfg(target_os = "macos")]
pub const MACOS_APP_ICON_PNG: &[u8] = include_bytes!("../assets/icons/laralah-macos.png");

pub struct EmbeddedAssets;

impl AssetSource for EmbeddedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "icons/app.png" => Some(APP_ICON_PNG),
            "icons/play.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><polygon points="6 4 20 12 6 20 6 4"/></svg>"#,
            ),
            "icons/folder.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z"/></svg>"#,
            ),
            "icons/server.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="6" rx="2"/><rect x="3" y="14" width="18" height="6" rx="2"/><line x1="7" y1="7" x2="7.01" y2="7"/><line x1="7" y1="17" x2="7.01" y2="17"/></svg>"#,
            ),
            "icons/layers.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><polygon points="12 3 2 8 12 13 22 8 12 3"/><polyline points="2 13 12 18 22 13"/><polyline points="2 17.5 12 22.5 22 17.5"/></svg>"#,
            ),
            "icons/clock.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><polyline points="12 7 12 12 15.5 14"/></svg>"#,
            ),
            "icons/bookmark.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M6 4h12a1 1 0 0 1 1 1v16l-7-4-7 4V5a1 1 0 0 1 1-1z"/></svg>"#,
            ),
            "icons/file.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="8" y1="13" x2="16" y2="13"/><line x1="8" y1="17" x2="13" y2="17"/></svg>"#,
            ),
            "icons/terminal.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>"#,
            ),
            "icons/code.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><polyline points="9.5 9.5 7 12 9.5 14.5"/><polyline points="14.5 9.5 17 12 14.5 14.5"/></svg>"#,
            ),
            "icons/table.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><line x1="3" y1="9.5" x2="21" y2="9.5"/><line x1="3" y1="14.75" x2="21" y2="14.75"/><line x1="9.5" y1="9.5" x2="9.5" y2="20"/></svg>"#,
            ),
            "icons/eye.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/></svg>"#,
            ),
            "icons/bolt.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><polygon points="13 2 4 14 11 14 10 22 20 10 13 10 13 2"/></svg>"#,
            ),
            "icons/symbol-class.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="6" r="3"/><circle cx="18" cy="18" r="3"/><path d="M8.59 8.59L15.42 15.42"/><path d="M15 6h3a2 2 0 0 1 2 2v3"/></svg>"#,
            ),
            "icons/symbol-method.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/></svg>"#,
            ),
            "icons/symbol-var.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="6" width="16" height="12" rx="2"/><line x1="8" y1="12" x2="16" y2="12"/></svg>"#,
            ),
            "icons/settings.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>"#,
            ),
            "icons/sidebar.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><line x1="9" y1="4" x2="9" y2="20"/></svg>"#,
            ),
            "icons/home.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M3 10.5L12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1v-9.5z"/></svg>"#,
            ),
            "icons/database.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v14c0 1.66 3.58 3 8 3s8-1.34 8-3V5"/><path d="M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3"/></svg>"#,
            ),
            "icons/plus.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>"#,
            ),
            "icons/search.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><line x1="20" y1="20" x2="16.05" y2="16.05"/></svg>"#,
            ),
            "icons/close.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>"#,
            ),
            "icons/sparkles.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/></svg>"#,
            ),
            "icons/sidebar-right.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><line x1="15" y1="4" x2="15" y2="20"/></svg>"#,
            ),
            "icons/sliders.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="7" x2="20" y2="7"/><line x1="4" y1="12" x2="20" y2="12"/><line x1="4" y1="17" x2="20" y2="17"/><circle cx="9" cy="7" r="2" fill="black"/><circle cx="15" cy="12" r="2" fill="black"/><circle cx="10" cy="17" r="2" fill="black"/></svg>"#,
            ),
            "icons/refresh.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>"#,
            ),
            "icons/send.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="19" x2="12" y2="5"/><polyline points="5 12 12 5 19 12"/></svg>"#,
            ),
            "icons/stop.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>"#,
            ),
            "icons/trash.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/></svg>"#,
            ),
            "icons/agent-claude.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black" fill-rule="evenodd"><path d="M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z"/></svg>"#,
            ),
            "icons/agent-codex.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black" fill-rule="evenodd"><path d="M9.205 8.658v-2.26c0-.19.072-.333.238-.428l4.543-2.616c.619-.357 1.356-.523 2.117-.523 2.854 0 4.662 2.212 4.662 4.566 0 .167 0 .357-.024.547l-4.71-2.759a.797.797 0 00-.856 0l-5.97 3.473zm10.609 8.8V12.06c0-.333-.143-.57-.429-.737l-5.97-3.473 1.95-1.118a.433.433 0 01.476 0l4.543 2.617c1.309.76 2.189 2.378 2.189 3.948 0 1.808-1.07 3.473-2.76 4.163zM7.802 12.703l-1.95-1.142c-.167-.095-.239-.238-.239-.428V5.899c0-2.545 1.95-4.472 4.591-4.472 1 0 1.927.333 2.712.928L8.23 5.067c-.285.166-.428.404-.428.737v6.898zM12 15.128l-2.795-1.57v-3.33L12 8.658l2.795 1.57v3.33L12 15.128zm1.796 7.23c-1 0-1.927-.332-2.712-.927l4.686-2.712c.285-.166.428-.404.428-.737v-6.898l1.974 1.142c.167.095.238.238.238.428v5.233c0 2.545-1.974 4.472-4.614 4.472zm-5.637-5.303l-4.544-2.617c-1.308-.761-2.188-2.378-2.188-3.948A4.482 4.482 0 014.21 6.327v5.423c0 .333.143.571.428.738l5.947 3.449-1.95 1.118a.432.432 0 01-.476 0zm-.262 3.9c-2.688 0-4.662-2.021-4.662-4.519 0-.19.024-.38.047-.57l4.686 2.71c.286.167.571.167.856 0l5.97-3.448v2.26c0 .19-.07.333-.237.428l-4.543 2.616c-.619.357-1.356.523-2.117.523zm5.899 2.83a5.947 5.947 0 005.827-4.756C22.287 18.339 24 15.84 24 13.296c0-1.665-.713-3.282-1.998-4.448.119-.5.19-.999.19-1.498 0-3.401-2.759-5.947-5.946-5.947-.642 0-1.26.095-1.88.31A5.962 5.962 0 0010.205 0a5.947 5.947 0 00-5.827 4.757C1.713 5.447 0 7.945 0 10.49c0 1.666.713 3.283 1.998 4.448-.119.5-.19 1-.19 1.499 0 3.401 2.759 5.946 5.946 5.946.642 0 1.26-.095 1.88-.309a5.96 5.96 0 004.162 1.713z"/></svg>"#,
            ),
            "icons/agent-agy.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="10 14 92 86" fill="black"><path d="M89.6992 93.695C94.3659 97.195 101.366 94.8617 94.9492 88.445C75.6992 69.7783 79.7825 18.445 55.8659 18.445C31.9492 18.445 36.0325 69.7783 16.7825 88.445C9.78251 95.445 17.3658 97.195 22.0325 93.695C40.1159 81.445 38.9492 59.8617 55.8659 59.8617C72.7825 59.8617 71.6159 81.445 89.6992 93.695Z"/></svg>"#,
            ),
            "icons/agent-qoder.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black" fill-rule="evenodd"><path d="M23.376 14.458v-4.056c0-2.304-1.003-4.154-2.748-5.075L11.612.574l-.046.086-.045.086c1.68.886 2.644 2.673 2.644 4.902v4.056a7.928 7.928 0 01-.014.454l-.005.061c-.005.081-.01.164-.018.245a4.897 4.897 0 01-.011.1l-.01.076c-.008.068-.015.135-.025.203l-.018.113-.01.058a9.99 9.99 0 01-.098.513l-.007.03a7.209 7.209 0 01-.074.294l-.024.086c-.027.099-.056.197-.087.296l-.027.085a9.592 9.592 0 01-.111.323l-.033.085-.018.046c-.032.082-.064.166-.098.248-.019.048-.04.096-.061.145l-.007.017a6 6 0 01-.084.187c-.024.056-.05.11-.077.165-.03.061-.058.122-.089.182a9.423 9.423 0 01-.176.332c-.03.056-.062.111-.094.167-.031.053-.062.108-.095.16-.033.055-.066.11-.101.164-.033.053-.065.104-.1.155-.034.055-.07.107-.111.169l-.099.144a15.193 15.193 0 01-.34.457c-.04.05-.08.102-.121.151l-.107.128-.007.008a6.987 6.987 0 01-.262.298l-.149.16-.116.12a9.562 9.562 0 01-.204.198l-.03.03-.072.069a9.05 9.05 0 01-.263.235l-.025.022-.029.026-.042.035a11.7 11.7 0 01-.22.18l-.07.055-.018.013a8.904 8.904 0 01-.194.146c-.029.02-.057.042-.086.063a7.7 7.7 0 01-.22.152l-.057.04a8.865 8.865 0 01-.293.185l-.062.037a10.424 10.424 0 01-.307.173l-.037.02-.196.103-.108.052-.012.006a6.196 6.196 0 01-.315.143c-.065.028-.13.054-.196.08l-.035.014-.086.034c-.07.026-.143.05-.215.075l-.039.014-.064.023a8.056 8.056 0 01-.323.097l-.63.173a7.285 7.285 0 01-.33.08l-.07.015c-.053.012-.104.023-.157.032l-.065.011-.085.015a2.332 2.332 0 01-.194.027l-.085.01a4.715 4.715 0 01-.16.018l-.034.003a4.861 4.861 0 01-.246.016h-.033a2.714 2.714 0 01-.155.005h-.106a3.384 3.384 0 01-.225-.007H4.86l-.15-.012-.066-.006a5.586 5.586 0 01-.187-.02l-.04-.005a5.14 5.14 0 01-.219-.035l-.054-.01a6.943 6.943 0 01-.347-.082l-.03-.008-.038-.01a5.034 5.034 0 01-.269-.086l-.063-.023a4.216 4.216 0 01-.188-.073l-.071-.031-.016-.007a4.959 4.959 0 01-.16-.074l-.026-.013a.164.164 0 00-.014-.007l-.671-.351.486.486h.016l8.995 4.742.093.048.03.014.02.01c.056.026.111.052.169.076l.016.008.073.032.195.076.022.008a.718.718 0 01.03.012l.014.004a4.693 4.693 0 00.323.1l.027.007c.073.02.147.038.22.055l.018.004.027.006.066.013.088.016c.075.014.15.026.226.038l.042.004c.064.009.128.016.193.022l.126.012.06.003.05.002.098.005c.046.002.094.002.14.003h.12c.05 0 .1-.002.161-.005h.033l.07-.004a6.17 6.17 0 00.184-.014l.033-.003a.753.753 0 00.058-.005l.108-.012.081-.01.08-.01.129-.021.082-.014.071-.012c.054-.01.107-.021.16-.033l.066-.013.059-.013c.096-.022.192-.046.287-.073l.63-.172a7.354 7.354 0 00.397-.122l.04-.015c.075-.025.149-.051.222-.078.032-.011.062-.024.093-.037l.03-.012c.067-.027.135-.054.2-.082l.128-.056.195-.09.021-.01.102-.05c.068-.034.135-.069.202-.105l.037-.02.073-.038c.08-.044.16-.092.24-.139l.026-.015a8.086 8.086 0 00.322-.2l.065-.045 1.98.902a1.748 1.748 0 002.472-1.59v-7.33l.004.004z" fill-opacity="0.48"/><path d="M11.617.576a3.904 3.904 0 00-.093-.047c-.016-.009-.033-.016-.05-.024a5.854 5.854 0 00-.166-.077l-.09-.04a4.094 4.094 0 00-.194-.074c-.017-.006-.035-.015-.053-.02l-.013-.005a5.18 5.18 0 00-.277-.088l-.07-.019a4.034 4.034 0 00-.219-.053c-.015-.003-.03-.008-.044-.012L10.253.1l-.057-.011a5.177 5.177 0 00-.225-.036L9.928.047a5.972 5.972 0 00-.191-.022L9.669.02a1.33 1.33 0 00-.058-.005L9.515.009a4.058 4.058 0 00-.111-.005C9.354 0 9.304 0 9.254 0h-.109c-.052 0-.106.004-.16.004L8.884.01A5.32 5.32 0 008.7.022l-.083.006h-.008c-.05.005-.1.013-.15.019l-.12.014c-.056.008-.112.018-.169.028-.037.006-.074.011-.11.018a7.054 7.054 0 00-.572.133l-.63.172c-.111.031-.22.064-.33.1-.037.011-.072.025-.108.037a12.91 12.91 0 00-.345.126c-.067.027-.133.054-.2.083l-.128.055a11.916 11.916 0 00-.318.15 7.376 7.376 0 00-.311.163c-.082.045-.16.092-.24.14a9.424 9.424 0 00-.35.218l-.016.01-.06.04a9.242 9.242 0 00-.51.37 12.54 12.54 0 00-.315.254l-.043.034-.01.007-.045.041c-.09.078-.18.158-.268.24l-.106.101c-.07.067-.139.135-.207.204l-.056.054-.063.067-.153.164-.12.133-.148.17c-.024.03-.049.057-.073.086l-.043.053a5.96 5.96 0 00-.123.155l-.118.151c-.04.053-.08.106-.118.16l-.075.101-.038.056-.107.155-.11.164a9.91 9.91 0 00-.168.265c-.012.02-.023.043-.037.063a12.43 12.43 0 00-.192.335l-.09.168c-.019.034-.04.07-.057.105-.011.022-.02.043-.032.065l-.092.188-.08.167a19.15 19.15 0 00-.083.192c-.017.038-.035.076-.05.115-.008.016-.014.034-.021.051-.035.083-.067.167-.1.253l-.051.134c-.04.11-.077.22-.113.33l-.02.057c0 .002 0 .005-.002.007l-.008.026c-.032.1-.06.2-.09.301l-.023.089c-.027.1-.052.2-.075.301l-.007.03a7.63 7.63 0 00-.057.267l-.008.048c-.015.074-.026.148-.038.223L.082 8.4c-.011.078-.02.156-.029.234-.006.051-.013.103-.017.154a6.57 6.57 0 00-.02.26c-.003.044-.007.086-.009.13-.004.128-.007.257-.007.386v4.056c0 1.478.42 2.741 1.138 3.692A4.75 4.75 0 002.73 18.67l9.015 4.753c-1.656-.874-2.728-2.685-2.73-5.051v-4.056c0-.13.004-.26.01-.39.002-.043.006-.085.01-.128.005-.088.01-.174.019-.261l.017-.155c.01-.077.018-.155.029-.234l.026-.164c.013-.074.025-.15.039-.223.02-.105.04-.21.064-.313l.008-.031c.023-.1.048-.201.075-.301l.023-.088c.028-.1.058-.202.09-.302l.008-.025.021-.063c.036-.11.073-.22.113-.33l.052-.134c.031-.085.065-.169.1-.253.022-.056.047-.111.07-.166a13.856 13.856 0 01.164-.358c.03-.063.06-.126.092-.188l.088-.172a9.22 9.22 0 01.187-.338l.096-.164c.034-.057.069-.112.104-.168l.1-.159a10.49 10.49 0 01.567-.786l.123-.155.116-.139c.05-.057.098-.115.148-.171a14.092 14.092 0 01.272-.297 9.706 9.706 0 01.432-.425c.088-.083.177-.163.268-.241l.054-.048a10.08 10.08 0 01.553-.435l.092-.068c.075-.053.15-.105.226-.156.019-.013.038-.028.06-.04a9.18 9.18 0 01.362-.227c.08-.047.16-.094.241-.139l.11-.058a7.643 7.643 0 01.521-.256l.126-.056a7.509 7.509 0 01.546-.208l.107-.037c.11-.036.22-.069.33-.1l.63-.172c.095-.026.191-.05.287-.072l.097-.02c.062-.014.125-.029.187-.04l.114-.018c.055-.01.11-.02.166-.028.04-.006.08-.01.12-.014.053-.007.105-.014.157-.019l.083-.006c.061-.005.123-.01.184-.013.034-.003.067-.003.101-.004l.16-.006h.11a3.187 3.187 0 01.261.008l.153.01.067.007c.064.006.128.013.192.022l.043.005a5.232 5.232 0 01.281.047l.141.03c.073.016.146.035.218.053l.07.019c.094.026.187.055.278.087l.067.025a4.326 4.326 0 01.449.19l.143.072L11.617.576z"/></svg>"#,
            ),
            "icons/agent-opencode.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="80 80 352 352" fill="none"><path d="M320 224V352H192V224H320Z" fill="black" fill-opacity="0.42"/><path fill-rule="evenodd" clip-rule="evenodd" d="M384 416H128V96H384V416ZM320 160H192V352H320V160Z" fill="black"/></svg>"#,
            ),
            "icons/check.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="5 12.5 10 17.5 19 7"/></svg>"#,
            ),
            "icons/chevron-down.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>"#,
            ),
            "icons/chevron-right.svg" => Some(
                br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 6 15 12 9 18"/></svg>"#,
            ),
            _ => None,
        };
        Ok(bytes.map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![
            "icons/app.png".into(),
            "icons/play.svg".into(),
            "icons/folder.svg".into(),
            "icons/server.svg".into(),
            "icons/layers.svg".into(),
            "icons/clock.svg".into(),
            "icons/bookmark.svg".into(),
            "icons/file.svg".into(),
            "icons/terminal.svg".into(),
            "icons/code.svg".into(),
            "icons/table.svg".into(),
            "icons/eye.svg".into(),
            "icons/bolt.svg".into(),
            "icons/symbol-class.svg".into(),
            "icons/symbol-method.svg".into(),
            "icons/symbol-var.svg".into(),
            "icons/settings.svg".into(),
            "icons/sidebar.svg".into(),
            "icons/home.svg".into(),
            "icons/database.svg".into(),
            "icons/plus.svg".into(),
            "icons/search.svg".into(),
            "icons/close.svg".into(),
            "icons/sparkles.svg".into(),
            "icons/sidebar-right.svg".into(),
            "icons/sliders.svg".into(),
            "icons/refresh.svg".into(),
            "icons/send.svg".into(),
            "icons/stop.svg".into(),
            "icons/trash.svg".into(),
            "icons/agent-claude.svg".into(),
            "icons/agent-codex.svg".into(),
            "icons/agent-agy.svg".into(),
            "icons/agent-qoder.svg".into(),
            "icons/agent-opencode.svg".into(),
            "icons/check.svg".into(),
            "icons/chevron-down.svg".into(),
            "icons/chevron-right.svg".into(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_app_icon_loads_valid_png() {
        let loaded = EmbeddedAssets
            .load("icons/app.png")
            .expect("asset load should succeed")
            .expect("icons/app.png should exist");
        assert!(loaded.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(loaded.as_ref(), APP_ICON_PNG);

        let listed = EmbeddedAssets.list("").expect("asset list should succeed");
        assert!(listed.iter().any(|p| p.as_ref() == "icons/app.png"));
        for icon in [
            "icons/sparkles.svg",
            "icons/sidebar-right.svg",
            "icons/sliders.svg",
            "icons/agent-claude.svg",
            "icons/agent-codex.svg",
            "icons/agent-agy.svg",
            "icons/agent-qoder.svg",
            "icons/agent-opencode.svg",
            "icons/check.svg",
            "icons/chevron-down.svg",
            "icons/chevron-right.svg",
        ] {
            assert!(
                EmbeddedAssets.load(icon).unwrap().is_some(),
                "missing embedded icon {icon}"
            );
        }
    }
}
