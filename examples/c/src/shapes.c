#include <math.h>
#include "../include/shapes.h"

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

double circle_area(const Circle *c) {
    return M_PI * c->radius * c->radius;
}

double circle_perimeter(const Circle *c) {
    return 2.0 * M_PI * c->radius;
}

double rect_area(const Rect *r) {
    return r->width * r->height;
}

int point_in_circle(const Point *p, const Circle *c) {
    double dist = point_distance(p, &c->center);
    return dist <= c->radius;
}

double point_distance(const Point *a, const Point *b) {
    double dx = a->x - b->x;
    double dy = a->y - b->y;
    return sqrt(dx * dx + dy * dy);
}
