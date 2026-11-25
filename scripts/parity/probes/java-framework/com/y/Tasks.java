package com.y;

import org.springframework.scheduling.annotation.Scheduled;

public class Tasks {
    @Scheduled(cron = "0 0 * * * *")
    public void sync() {
    }
}
