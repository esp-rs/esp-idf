(function() {
    const implementors = Object.fromEntries([["embedded_hal",[]],["esp_idf_hal",[["impl&lt;'d, DEVICE, DRIVER&gt; <a class=\"trait\" href=\"embedded_hal/spi/trait.SpiDevice.html\" title=\"trait embedded_hal::spi::SpiDevice\">SpiDevice</a> for <a class=\"struct\" href=\"esp_idf_hal/spi/struct.SpiSoftCsDeviceDriver.html\" title=\"struct esp_idf_hal::spi::SpiSoftCsDeviceDriver\">SpiSoftCsDeviceDriver</a>&lt;'d, DEVICE, DRIVER&gt;<div class=\"where\">where\n    DEVICE: Borrow&lt;<a class=\"struct\" href=\"esp_idf_hal/spi/struct.SpiSharedDeviceDriver.html\" title=\"struct esp_idf_hal::spi::SpiSharedDeviceDriver\">SpiSharedDeviceDriver</a>&lt;'d, DRIVER&gt;&gt; + 'd,\n    DRIVER: Borrow&lt;<a class=\"struct\" href=\"esp_idf_hal/spi/struct.SpiDriver.html\" title=\"struct esp_idf_hal::spi::SpiDriver\">SpiDriver</a>&lt;'d&gt;&gt; + 'd,</div>",0],["impl&lt;'d, T&gt; <a class=\"trait\" href=\"embedded_hal/spi/trait.SpiDevice.html\" title=\"trait embedded_hal::spi::SpiDevice\">SpiDevice</a> for <a class=\"struct\" href=\"esp_idf_hal/spi/struct.SpiDeviceDriver.html\" title=\"struct esp_idf_hal::spi::SpiDeviceDriver\">SpiDeviceDriver</a>&lt;'d, T&gt;<div class=\"where\">where\n    T: Borrow&lt;<a class=\"struct\" href=\"esp_idf_hal/spi/struct.SpiDriver.html\" title=\"struct esp_idf_hal::spi::SpiDriver\">SpiDriver</a>&lt;'d&gt;&gt; + 'd,</div>",0]]]]);
    if (window.register_implementors) {
        window.register_implementors(implementors);
    } else {
        window.pending_implementors = implementors;
    }
})()
//{"start":59,"fragment_lengths":[19,1292]}