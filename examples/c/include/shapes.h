#ifndef SHAPES_H
#define SHAPES_H

/** A 2D point. */
typedef struct {
    double x;
    double y;
} Point;

/** A circle defined by center and radius. */
typedef struct {
    Point center;
    double radius;
} Circle;

/** A rectangle defined by origin and dimensions. */
typedef struct {
    Point origin;
    double width;
    double height;
} Rect;

/** Compute the area of a circle. */
double circle_area(const Circle *c);

/** Compute the perimeter of a circle. */
double circle_perimeter(const Circle *c);

/** Compute the area of a rectangle. */
double rect_area(const Rect *r);

/** Check if a point is inside a circle. */
int point_in_circle(const Point *p, const Circle *c);

/** Compute distance between two points. */
double point_distance(const Point *a, const Point *b);

#endif
