(function() {
    var type_impls = Object.fromEntries([["embedded_svc",[]],["esp_idf_svc",[]]]);
    if (window.register_type_impls) {
        window.register_type_impls(type_impls);
    } else {
        window.pending_type_impls = type_impls;
    }
})()
//{"start":55,"fragment_lengths":[19,19]}