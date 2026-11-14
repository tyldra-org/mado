//! Every setting, declared once (spec 06 §1, §8). Labels, help and search
//! keywords live in `ui/src/i18n/en.json` under `settings.<key>.label`,
//! `.help` and `.keywords`.
//!
//! Not here yet: per-command keyboard shortcuts (generated when the menus
//! land in M2-T15), `translate.offer` and the v1.1 features, and per-profile
//! name, colour and avatar, which live in the app database's `profiles`
//! table (spec 12).

use crate::setting::settings;
#[allow(unused_imports)]
use crate::types::{Restart, SettingType};

settings! {
    StartupMode: String {
        key: "startup.mode",
        ty: SettingType::Enum(&["newTab", "continue", "pages"]),
        default: r#""newTab""#,
        scope: Profile,
        section: General,
    }
    StartupPages: Vec<String> {
        key: "startup.pages",
        ty: SettingType::List(&SettingType::Url),
        default: r#"[]"#,
        scope: Profile,
        section: General,
    }
    GeneralExternalLinks: String {
        key: "general.externalLinks",
        ty: SettingType::Enum(&["newTab", "newWindow", "currentTab"]),
        default: r#""newTab""#,
        scope: Profile,
        section: General,
    }
    GeneralWarnBeforeQuit: bool {
        key: "general.warnBeforeQuit",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: App,
        section: General,
    }
    GeneralWarnUnsavedText: bool {
        key: "general.warnUnsavedText",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: Profile,
        section: General,
    }
    GeneralOpenAtLogin: bool {
        key: "general.openAtLogin",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: App,
        section: General,
    }
    AppShowProfilePicker: bool {
        key: "app.showProfilePicker",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: App,
        section: General,
    }
    AppearanceTheme: String {
        key: "appearance.theme",
        ty: SettingType::Enum(&["system", "light", "dark"]),
        default: r#""system""#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceLayout: String {
        key: "appearance.layout",
        ty: SettingType::Enum(&["vertical", "horizontal"]),
        default: r#""vertical""#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceDensity: String {
        key: "appearance.density",
        ty: SettingType::Enum(&["comfortable", "compact"]),
        default: r#""comfortable""#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceToolbarButtons: Vec<String> {
        key: "appearance.toolbarButtons",
        ty: SettingType::List(&SettingType::String { max_len: 64 }),
        default: r#"[]"#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceBookmarksBar: bool {
        key: "appearance.bookmarksBar",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceShowHomeButton: bool {
        key: "appearance.showHomeButton",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceHomePage: String {
        key: "appearance.homePage",
        ty: SettingType::Url,
        default: r#""mado://newtab""#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceFontsStandard: String {
        key: "appearance.fonts.standard",
        ty: SettingType::String { max_len: 128 },
        default: r#""""#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceFontsSerif: String {
        key: "appearance.fonts.serif",
        ty: SettingType::String { max_len: 128 },
        default: r#""""#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceFontsSansSerif: String {
        key: "appearance.fonts.sansSerif",
        ty: SettingType::String { max_len: 128 },
        default: r#""""#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceFontsFixed: String {
        key: "appearance.fonts.fixed",
        ty: SettingType::String { max_len: 128 },
        default: r#""""#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceMinimumFontSize: i64 {
        key: "appearance.minimumFontSize",
        ty: SettingType::Int { min: 0, max: 24, step: 1 },
        default: r#"0"#,
        scope: Profile,
        section: Appearance,
    }
    AppearanceDefaultZoom: f64 {
        key: "appearance.defaultZoom",
        ty: SettingType::Float { min: 0.25, max: 5.0 },
        default: r#"1.0"#,
        scope: Profile,
        section: Appearance,
    }
    AddressBarShow: String {
        key: "addressBar.show",
        ty: SettingType::Enum(&["site", "fullUrl"]),
        default: r#""site""#,
        scope: Profile,
        section: Appearance,
    }
    TabsNewTabPosition: String {
        key: "tabs.newTabPosition",
        ty: SettingType::Enum(&["end", "afterCurrent"]),
        default: r#""end""#,
        scope: Profile,
        section: Tabs,
    }
    TabsClosePinned: String {
        key: "tabs.closePinned",
        ty: SettingType::Enum(&["unload", "close"]),
        default: r#""unload""#,
        scope: Profile,
        section: Tabs,
    }
    TabsCtrlTabOrder: String {
        key: "tabs.ctrlTabOrder",
        ty: SettingType::Enum(&["strip", "recent"]),
        default: r#""strip""#,
        scope: Profile,
        section: Tabs,
    }
    TabsMemorySaver: String {
        key: "tabs.memorySaver",
        ty: SettingType::Enum(&["off", "moderate", "balanced", "maximum"]),
        default: r#""balanced""#,
        scope: Profile,
        section: Tabs,
    }
    TabsSleepAfter: i64 {
        key: "tabs.sleepAfter",
        ty: SettingType::Duration { min: 60_000, max: 86_400_000 },
        default: r#"1800000"#,
        scope: Profile,
        section: Tabs,
    }
    TabsKeepAwakeSites: Vec<String> {
        key: "tabs.keepAwakeSites",
        ty: SettingType::List(&SettingType::String { max_len: 253 }),
        default: r#"[]"#,
        scope: Profile,
        section: Tabs,
    }
    TabsAutoPip: bool {
        key: "tabs.autoPip",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Tabs,
    }
    PerformanceEnergySaver: String {
        key: "performance.energySaver",
        ty: SettingType::Enum(&["off", "battery20", "unplugged"]),
        default: r#""battery20""#,
        scope: Profile,
        section: Tabs,
    }
    SearchDefaultEngine: String {
        key: "search.defaultEngine",
        ty: SettingType::String { max_len: 64 },
        default: r#""""#,
        scope: Profile,
        section: Search,
    }
    SearchSuggestions: String {
        key: "search.suggestions",
        ty: SettingType::Enum(&["asked", "on", "off"]),
        default: r#""asked""#,
        scope: Profile,
        section: Search,
    }
    SearchSuggestionsInPrivate: bool {
        key: "search.suggestionsInPrivate",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Search,
    }
    SafetyDangerousSiteWarnings: bool {
        key: "safety.dangerousSiteWarnings",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: Profile,
        section: Privacy,
        protected: true,
        undo_notice: true,
    }
    SecurityHttpsFirst: String {
        key: "security.httpsFirst",
        ty: SettingType::Enum(&["off", "upgrade", "strict"]),
        default: r#""upgrade""#,
        scope: Profile,
        section: Privacy,
    }
    PrivacyTrackingPrevention: String {
        key: "privacy.trackingPrevention",
        ty: SettingType::Enum(&["off", "balanced", "strict"]),
        default: r#""balanced""#,
        scope: Profile,
        section: Privacy,
    }
    PrivacyGpc: bool {
        key: "privacy.gpc",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: Profile,
        section: Privacy,
    }
    PrivacyWebrtcHideIp: bool {
        key: "privacy.webrtcHideIp",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: Profile,
        section: Privacy,
    }
    PrivacyClearOnExit: Vec<String> {
        key: "privacy.clearOnExit",
        ty: SettingType::List(&SettingType::Enum(&["cookies", "cache", "history", "downloads", "siteData"])),
        default: r#"[]"#,
        scope: Profile,
        section: Privacy,
    }
    PrivacyLockPrivateWindows: bool {
        key: "privacy.lockPrivateWindows",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Privacy,
    }
    HistoryRetention: i64 {
        key: "history.retention",
        ty: SettingType::Duration { min: 86_400_000, max: 315_360_000_000 },
        default: r#"31536000000"#,
        scope: Profile,
        section: Privacy,
    }
    HistoryIndexPageText: bool {
        key: "history.indexPageText",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: Profile,
        section: Privacy,
    }
    HistoryIndexLimitGb: i64 {
        key: "history.indexLimitGb",
        ty: SettingType::Int { min: 1, max: 100, step: 1 },
        default: r#"2"#,
        scope: Profile,
        section: Privacy,
    }
    BlockerEnabled: bool {
        key: "blocker.enabled",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: Profile,
        section: Privacy,
        protected: true,
        undo_notice: true,
    }
    BlockerLists: Vec<String> {
        key: "blocker.lists",
        ty: SettingType::List(&SettingType::String { max_len: 128 }),
        default: r#"[]"#,
        scope: Profile,
        section: Privacy,
    }
    PermissionsCamera: String {
        key: "permissions.camera",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""ask""#,
        scope: Profile,
        section: SiteSettings,
        protected: true,
    }
    PermissionsMicrophone: String {
        key: "permissions.microphone",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""ask""#,
        scope: Profile,
        section: SiteSettings,
        protected: true,
    }
    PermissionsScreen: String {
        key: "permissions.screen",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""ask""#,
        scope: Profile,
        section: SiteSettings,
        protected: true,
    }
    PermissionsLocation: String {
        key: "permissions.location",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""ask""#,
        scope: Profile,
        section: SiteSettings,
        protected: true,
    }
    PermissionsNotifications: String {
        key: "permissions.notifications",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""ask""#,
        scope: Profile,
        section: SiteSettings,
    }
    PermissionsClipboardRead: String {
        key: "permissions.clipboardRead",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""ask""#,
        scope: Profile,
        section: SiteSettings,
    }
    PermissionsSpeechRecognition: String {
        key: "permissions.speechRecognition",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""ask""#,
        scope: Profile,
        section: SiteSettings,
    }
    PermissionsPopups: String {
        key: "permissions.popups",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""block""#,
        scope: Profile,
        section: SiteSettings,
    }
    PermissionsAutomaticDownloads: String {
        key: "permissions.automaticDownloads",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""ask""#,
        scope: Profile,
        section: SiteSettings,
    }
    PermissionsSound: String {
        key: "permissions.sound",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""allow""#,
        scope: Profile,
        section: SiteSettings,
    }
    PermissionsJavascript: String {
        key: "permissions.javascript",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""allow""#,
        scope: Profile,
        section: SiteSettings,
    }
    PermissionsImages: String {
        key: "permissions.images",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""allow""#,
        scope: Profile,
        section: SiteSettings,
    }
    PermissionsInsecureContent: String {
        key: "permissions.insecureContent",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""block""#,
        scope: Profile,
        section: SiteSettings,
    }
    PermissionsAutomaticFullscreen: String {
        key: "permissions.automaticFullscreen",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""block""#,
        scope: Profile,
        section: SiteSettings,
    }
    PermissionsProtectedContent: String {
        key: "permissions.protectedContent",
        ty: SettingType::Enum(&["ask", "allow", "block"]),
        default: r#""allow""#,
        scope: Profile,
        section: SiteSettings,
    }
    PasswordsWhoFills: String {
        key: "passwords.whoFills",
        ty: SettingType::Enum(&["automatic", "mado", "onePassword", "bitwarden", "extension"]),
        default: r#""automatic""#,
        scope: Profile,
        section: Passwords,
        protected: true,
    }
    VaultLockAfter: i64 {
        key: "vault.lockAfter",
        ty: SettingType::Duration { min: 60_000, max: 86_400_000 },
        default: r#"900000"#,
        scope: Profile,
        section: Passwords,
        protected: true,
    }
    VaultFillApprovalWindow: i64 {
        key: "vault.fillApprovalWindow",
        ty: SettingType::Duration { min: 0, max: 3_600_000 },
        default: r#"300000"#,
        scope: Profile,
        section: Passwords,
        protected: true,
    }
    VaultSaveDestinationLogins: String {
        key: "vault.saveDestination.logins",
        ty: SettingType::Enum(&["mado", "onePassword", "bitwarden"]),
        default: r#""mado""#,
        scope: Profile,
        section: Passwords,
        protected: true,
    }
    VaultOfferToSave: bool {
        key: "vault.offerToSave",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: Profile,
        section: Passwords,
        protected: true,
    }
    ExtensionsDeveloperMode: bool {
        key: "extensions.developerMode",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Extensions,
        protected: true,
    }
    ExtensionsOverlapPrompts: bool {
        key: "extensions.overlapPrompts",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: Profile,
        section: Extensions,
    }
    DownloadsFolder: String {
        key: "downloads.folder",
        ty: SettingType::Path,
        default: r#""""#,
        scope: Profile,
        section: Downloads,
    }
    DownloadsAskWhere: bool {
        key: "downloads.askWhere",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Downloads,
    }
    DownloadsOpenSafeTypes: String {
        key: "downloads.openSafeTypes",
        ty: SettingType::Enum(&["none", "safe"]),
        default: r#""none""#,
        scope: Profile,
        section: Downloads,
    }
    AiApprovalMode: String {
        key: "ai.approvalMode",
        ty: SettingType::Enum(&["ask", "auto", "full"]),
        default: r#""auto""#,
        scope: Profile,
        section: Ai,
        protected: true,
        feature: Some("ai"),
    }
    AiAgentsInUserTabs: String {
        key: "ai.agentsInUserTabs",
        ty: SettingType::Enum(&["never", "askFirst", "always"]),
        default: r#""askFirst""#,
        scope: Profile,
        section: Ai,
        protected: true,
        feature: Some("agents"),
    }
    AiAllowedSensitiveSites: Vec<String> {
        key: "ai.allowedSensitiveSites",
        ty: SettingType::List(&SettingType::String { max_len: 253 }),
        default: r#"[]"#,
        scope: Profile,
        section: Ai,
        protected: true,
        feature: Some("agents"),
    }
    AiSpendLimitsPerTask: serde_json::Value {
        key: "ai.spendLimits.perTask",
        ty: SettingType::Json { schema: r#"{"type":["integer","null"],"minimum":0}"# },
        default: r#"null"#,
        scope: Profile,
        section: Ai,
        protected: true,
        feature: Some("ai"),
    }
    AiSpendLimitsPerDay: serde_json::Value {
        key: "ai.spendLimits.perDay",
        ty: SettingType::Json { schema: r#"{"type":["integer","null"],"minimum":0}"# },
        default: r#"null"#,
        scope: Profile,
        section: Ai,
        protected: true,
        feature: Some("ai"),
    }
    AiChatRetention: String {
        key: "ai.chatRetention",
        ty: SettingType::Enum(&["day", "week", "month", "forever"]),
        default: r#""week""#,
        scope: Profile,
        section: Ai,
        feature: Some("ai"),
    }
    AiTaskTimeLimit: i64 {
        key: "ai.taskTimeLimit",
        ty: SettingType::Duration { min: 60_000, max: 86_400_000 },
        default: r#"1800000"#,
        scope: Profile,
        section: Ai,
        feature: Some("agents"),
    }
    AiCommandsCustom: Vec<serde_json::Value> {
        key: "ai.commands.custom",
        ty: SettingType::List(&SettingType::Json { schema: r#"{"type":"object"}"# }),
        default: r#"[]"#,
        scope: Profile,
        section: Ai,
        feature: Some("textCommands"),
    }
    AiHelpersPerTask: i64 {
        key: "ai.helpersPerTask",
        ty: SettingType::Int { min: 1, max: 6, step: 1 },
        default: r#"6"#,
        scope: Profile,
        section: Ai,
        feature: Some("agents"),
    }
    DeveloperControlEnabled: bool {
        key: "developer.controlEnabled",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Developers,
        protected: true,
    }
    DeveloperMcpEnabled: bool {
        key: "developer.mcpEnabled",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Developers,
        protected: true,
    }
    DeveloperInspectPages: bool {
        key: "developer.inspectPages",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: Profile,
        section: Developers,
        restart: Restart::Engine,
    }
    ProxyMode: String {
        key: "proxy.mode",
        ty: SettingType::Enum(&["system", "direct", "manual"]),
        default: r#""system""#,
        scope: Profile,
        section: Advanced,
    }
    ProxyHttp: String {
        key: "proxy.http",
        ty: SettingType::String { max_len: 253 },
        default: r#""""#,
        scope: Profile,
        section: Advanced,
    }
    ProxySocks5: String {
        key: "proxy.socks5",
        ty: SettingType::String { max_len: 253 },
        default: r#""""#,
        scope: Profile,
        section: Advanced,
    }
    ProxyBypass: Vec<String> {
        key: "proxy.bypass",
        ty: SettingType::List(&SettingType::String { max_len: 253 }),
        default: r#"[]"#,
        scope: Profile,
        section: Advanced,
    }
    LanguagesUi: String {
        key: "languages.ui",
        ty: SettingType::String { max_len: 35 },
        default: r#""system""#,
        scope: App,
        section: Advanced,
        restart: Restart::App,
    }
    LanguagesAccept: Vec<String> {
        key: "languages.accept",
        ty: SettingType::List(&SettingType::String { max_len: 35 }),
        default: r#"["system"]"#,
        scope: Profile,
        section: Advanced,
    }
    LanguagesSpellcheck: Vec<String> {
        key: "languages.spellcheck",
        ty: SettingType::List(&SettingType::String { max_len: 35 }),
        default: r#"[]"#,
        scope: Profile,
        section: Advanced,
    }
    UpdateChannel: String {
        key: "update.channel",
        ty: SettingType::Enum(&["stable", "beta", "nightly"]),
        default: r#""stable""#,
        scope: App,
        section: About,
    }
    DiagnosticsCrashReports: bool {
        key: "diagnostics.crashReports",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: App,
        section: About,
    }
    FeaturesAi: bool {
        key: "features.ai",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Features,
        protected: true,
    }
    FeaturesAgents: bool {
        key: "features.agents",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Features,
        protected: true,
    }
    FeaturesRoutines: bool {
        key: "features.routines",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Features,
        protected: true,
    }
    FeaturesMemory: bool {
        key: "features.memory",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Features,
        protected: true,
    }
    FeaturesTextCommands: bool {
        key: "features.textCommands",
        ty: SettingType::Bool,
        default: r#"false"#,
        scope: Profile,
        section: Features,
        protected: true,
    }
    FeaturesReader: bool {
        key: "features.reader",
        ty: SettingType::Bool,
        default: r#"true"#,
        scope: Profile,
        section: Features,
        protected: true,
    }
}
