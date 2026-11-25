package com.y;

import org.springframework.beans.factory.annotation.Value;
import org.springframework.boot.context.properties.ConfigurationProperties;

@ConfigurationProperties(prefix = "probe.api")
public class Settings {
    @Value("${probe.name}")
    private String name;
}
