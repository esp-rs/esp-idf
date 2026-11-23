(function() {
    var type_impls = Object.fromEntries([["esp_idf_hal",[]],["esp_idf_svc",[]],["esp_idf_sys",[]]]);
    if (window.register_type_impls) {
        window.register_type_impls(type_impls);
    } else {
        window.pending_type_impls = type_impls;
    }
})()
//{"start":55,"fragment_lengths":[18,19,19]}