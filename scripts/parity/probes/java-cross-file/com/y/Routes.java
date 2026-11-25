package com.y;

import org.springframework.web.reactive.function.server.HandlerFunction;
import org.springframework.web.reactive.function.server.RouterFunction;
import org.springframework.web.reactive.function.server.RouterFunctions;

public class Routes {
    public RouterFunction<?> routes(HandlerFunction<?> handler) {
        return RouterFunctions.route().GET("/probe", handler).build();
    }
}
