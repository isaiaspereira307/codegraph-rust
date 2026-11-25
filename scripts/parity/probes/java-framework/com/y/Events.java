package com.y;

import org.springframework.context.ApplicationEventPublisher;
import org.springframework.context.event.EventListener;

public class Events {
    private final ApplicationEventPublisher publisher;

    public Events(ApplicationEventPublisher publisher) {
        this.publisher = publisher;
    }

    public void publish() {
        publisher.publishEvent(new LocalEvent());
    }

    @EventListener(LocalEvent.class)
    public void on(LocalEvent event) {
    }
}
