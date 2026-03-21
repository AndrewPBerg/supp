#include <stdio.h>
#include "../include/shapes.h"

int main(void) {
    Point center = {0.0, 0.0};
    Circle c = {center, 5.0};
    Rect r = {{1.0, 2.0}, 10.0, 4.0};

    printf("Circle area: %.2f\n", circle_area(&c));
    printf("Circle perimeter: %.2f\n", circle_perimeter(&c));
    printf("Rect area: %.2f\n", rect_area(&r));

    Point test = {3.0, 4.0};
    if (point_in_circle(&test, &c)) {
        printf("Point (%.1f, %.1f) is inside the circle\n", test.x, test.y);
    }

    printf("Distance: %.2f\n", point_distance(&center, &test));
    return 0;
}
