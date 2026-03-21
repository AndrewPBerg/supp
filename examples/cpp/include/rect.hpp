#ifndef RECT_HPP
#define RECT_HPP

#include "shape.hpp"

/** A rectangle defined by origin, width, and height. */
class Rect : public Shape {
public:
    Rect(double x, double y, double w, double h);

    double area() const override;
    double perimeter() const override;
    std::string describe() const override;

    double getWidth() const;
    double getHeight() const;

private:
    double x_, y_, width_, height_;
};

#endif
