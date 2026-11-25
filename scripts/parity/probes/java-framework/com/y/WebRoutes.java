package com.y;

import static org.springframework.web.reactive.function.server.RouterFunctions.route;
import org.springframework.web.reactive.function.server.RouterFunction;
import org.springframework.web.reactive.function.server.ServerResponse;

public class WebRoutes {
    public RouterFunction<ServerResponse> routes(Handler handler) {
        return route()
            .GET("/web", handler::handle)
            .build();
    }
}
