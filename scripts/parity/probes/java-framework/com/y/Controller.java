package com.y;

import org.springframework.web.bind.annotation.GetMapping;

public class Controller {
    @GetMapping("/probe")
    public String probe() {
        return "ok";
    }
}
