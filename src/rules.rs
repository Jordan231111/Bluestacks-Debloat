//! Explicit, reviewable rules, observed on 5.22.265.1013. Unknown settings are
//! never guessed or created. Android packages use a separate exact allowlist.
pub const ADS: &[&str] = &[
    "bst.enable_programmatic_ads",
    "bst.enable_boot_banner",
    "bst.enable_android_ads_test_app",
    "bst.feature.programmatic_ads",
    "bst.feature.enable_boot_promotion_grid",
    "bst.feature.show_gp_ads",
];
pub const STATS: &[&str] = &[
    "bst.feature.android_ads_stats",
    "bst.feature.app_install_stats",
    "bst.feature.send_auto_record_stats",
    "bst.feature.send_internal_notification_stats",
    "bst.feature.send_notification_stats",
    "bst.feature.send_nowbux_login_boot_stats",
    "bst.feature.send_offer_stats",
    "bst.feature.send_programmatic_ads_boot_stats",
    "bst.feature.send_programmatic_ads_click_stats",
    "bst.feature.send_programmatic_ads_fill_stats",
    "bst.feature.send_usage_state_stats",
    "bst.feature.usage_stats",
];
pub const DOWNLOADS: &[&str] = &["bst.enable_smart_downloads", "bst.feature.smart_downloads"];
pub const CLOUD: &[&str] = &[
    "bst.feature.bluestacksX",
    "bst.enable_bsx_app_shortcuts",
    "bst.launch_store_on_boot",
    "bst.feature.show_cloud_instance",
    "bst.feature.nowgg_cloud_upload_enabled",
    "bst.feature.auto_upload_nowgg_recording",
    "bst.feature.auto_upload_nowgg_moments",
    "bst.enable_auto_upload_recording",
    "bst.feature.nowgg_login_popup",
    "bst.feature.nowbux",
    "bst.show_nowbux_rewards_red_dot_onboarding",
    "bst.feature.show_moments",
    "bst.enable_ai_highlights",
    "bst.feature.show_ai_highlights",
    "bst.ai.enabled",
    "bst.feature.blueai",
    "bst.feature.ai_chat",
    "bst.feature.popout_ai_chat",
    "bst.feature.creator_studio",
    "bst.feature.live_stream",
];
