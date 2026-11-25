package com.y;

import org.springframework.beans.factory.annotation.Value;

public record Config(@Value("${probe.name}") String name) {
}
