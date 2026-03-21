#ifndef SHAPE_HPP
#define SHAPE_HPP

#include <string>

/** Abstract base class for all shapes. */
class Shape {
public:
    virtual ~Shape() = default;

    /** Compute the area of this shape. */
    virtual double area() const = 0;

    /** Compute the perimeter of this shape. */
    virtual double perimeter() const = 0;

    /** Human-readable description. */
    virtual std::string describe() const = 0;
};

#endif
