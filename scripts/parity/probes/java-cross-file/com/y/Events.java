package com.y;

import org.springframework.kafka.annotation.KafkaListener;

public class Events {
    @KafkaListener(topics = "probe")
    public void on(String value) {
    }
}
